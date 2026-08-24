//! Owns refresh policy and the state the UI renders.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/UsageModel.swift`. Policy, in order:
//!  1. Read the status line cache. If it's fresh, use it and make no network call.
//!  2. Otherwise, if the OAuth fallback is on, poll the usage endpoint.
//!  3. If that fails, keep showing the last good snapshot and label it stale.
//!
//! Swift runs this on the main actor with an async `Task`. Here a single worker
//! thread owns the IO and the UI reads a mutex, which keeps every network call
//! off the frame loop and means `egui` never blocks on a socket.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::oauth::{Failure, OAuthUsageClient};
use crate::settings::{defaults, Settings};
use crate::snapshot::{UsageError, UsageSnapshot};
use crate::status_line_cache::{self, CacheFileWatcher, Context};

/// What a refresh should do, given what we already hold. Split out from the IO
/// so the whole ladder is testable without a network or a filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshPlan {
    /// The status line cache is current. Free, and the common case while a
    /// Claude Code session is open.
    CacheIsFresh,
    /// The fallback is off and we have a cache to show. Not an error.
    CacheOnly,
    /// The fallback is off and there is nothing to show.
    NoSource,
    /// We already hold something recent enough. Opening the popover calls
    /// refresh, so without this a handful of clicks would each become a request
    /// to an endpoint that rate limits hard.
    HeldRecently,
    /// A 429 put us in backoff and it hasn't elapsed.
    BackedOff,
    Poll,
}

/// `cached_age` is the age of the file we just read (None when there is no
/// cache file); `held_age` is the age of whatever the model already holds after
/// adopting it.
pub fn plan(
    cached_age: Option<f64>,
    held_age: Option<f64>,
    force: bool,
    oauth_enabled: bool,
    backoff_active: bool,
) -> RefreshPlan {
    if let Some(age) = cached_age {
        if age < defaults::CACHE_STALE_AFTER && !force {
            return RefreshPlan::CacheIsFresh;
        }
    }

    if !oauth_enabled {
        return if cached_age.is_none() { RefreshPlan::NoSource } else { RefreshPlan::CacheOnly };
    }

    if !force {
        if let Some(age) = held_age {
            if age < defaults::CACHE_STALE_AFTER {
                return RefreshPlan::HeldRecently;
            }
        }
        if backoff_active {
            return RefreshPlan::BackedOff;
        }
    }

    RefreshPlan::Poll
}

/// Exponential backoff, honouring `Retry-After` when the server sent one.
///
/// The exponential starts at `BACKOFF_BASE` and doubles per consecutive 429, so
/// the first strike costs a minute rather than the whole ceiling. A `Retry-After`
/// further out always wins -- the server knows when the window actually reopens
/// -- but one closer in can't undercut the exponential, which is the only thing
/// damping a server that keeps saying "try again in 5 seconds" and then 429s
/// again.
pub fn backoff_deadline(
    consecutive_rate_limits: u32,
    retry_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let exponent = consecutive_rate_limits.saturating_sub(1) as f64;
    let interval = (defaults::BACKOFF_BASE * 2f64.powf(exponent)).min(defaults::MAX_BACKOFF);
    let floor = now + chrono::Duration::milliseconds((interval * 1000.0) as i64);
    match retry_at {
        Some(retry_at) if retry_at > floor => retry_at,
        _ => floor,
    }
}

#[derive(Debug, Default)]
pub struct State {
    pub snapshot: Option<UsageSnapshot>,
    pub context: Option<Context>,
    pub last_error: Option<UsageError>,
    pub is_refreshing: bool,
    pub settings: Settings,
    backoff_until: Option<DateTime<Utc>>,
    consecutive_rate_limits: u32,
}

impl State {
    /// A state holding a fixed snapshot, for tests and for rendering the README
    /// images. Never used by the running app -- the mirror of Swift's
    /// `UsageModel.preview`.
    pub fn preview(snapshot: UsageSnapshot) -> Self {
        Self { snapshot: Some(snapshot), ..Self::default() }
    }

    /// Dim the indicator once data is old enough to mislead -- roughly two
    /// missed refreshes.
    pub fn is_stale(&self) -> bool {
        match &self.snapshot {
            None => true,
            Some(snapshot) => snapshot.age() > (self.settings.poll_interval_seconds * 2) as f64,
        }
    }

    /// The empty state already says everything `NoData` would, so showing both
    /// prints the same sentence twice.
    pub fn visible_error(&self) -> Option<&UsageError> {
        let error = self.last_error.as_ref()?;
        if self.snapshot.is_none() && *error == UsageError::NoData {
            return None;
        }
        Some(error)
    }

    fn adopt(&mut self, new: UsageSnapshot, new_context: Option<Context>) {
        if let Some(current) = &self.snapshot {
            if new.captured_at < current.captured_at {
                return;
            }
        }
        let mut adopted = new;
        // The plan badge only ever arrives from the OAuth path. Once we know
        // it, keep it -- otherwise it would blink out of the header on the next
        // status line write and back in on the next poll.
        if adopted.plan.is_none() {
            adopted.plan = self.snapshot.as_ref().and_then(|s| s.plan.clone());
        }
        self.snapshot = Some(adopted);
        if new_context.is_some() {
            self.context = new_context;
        }
    }
}

enum Wake {
    Tick,
    CacheChanged,
    Refresh { force: bool },
    SettingsChanged,
    Stop,
}

pub struct UsageModel {
    state: Arc<Mutex<State>>,
    sender: Sender<Wake>,
    worker: Option<std::thread::JoinHandle<()>>,
    _watcher: Option<CacheFileWatcher>,
}

impl UsageModel {
    /// `repaint` is called whenever state changes; the app hands it
    /// `egui::Context::request_repaint`.
    pub fn start(settings: Settings, repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let state = Arc::new(Mutex::new(State { settings, ..State::default() }));
        let (sender, receiver) = std::sync::mpsc::channel();

        // The watcher fires on a notify thread; all it does is post a wake-up.
        let watcher_sender = sender.clone();
        let watcher = CacheFileWatcher::start(move || {
            let _ = watcher_sender.send(Wake::CacheChanged);
        })
        .ok();

        let worker_state = Arc::clone(&state);
        let repaint = Arc::new(repaint);
        let worker = std::thread::Builder::new()
            .name("claude-quota-refresh".into())
            .spawn(move || run(worker_state, receiver, repaint))
            .ok();

        let model = Self { state, sender, worker, _watcher: watcher };
        model.refresh(false);
        model
    }

    pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn refresh(&self, force: bool) {
        let _ = self.sender.send(Wake::Refresh { force });
    }

    /// A manual refresh from the popover clears any backoff -- the user asked.
    pub fn refresh_now(&self) {
        {
            let mut state = self.state();
            state.backoff_until = None;
            state.consecutive_rate_limits = 0;
        }
        self.refresh(true);
    }

    /// Persist and apply a settings change. Turning the fallback on refreshes
    /// straight away, the way flipping the switch on macOS does.
    pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) {
        let (settings, enabled_fallback) = {
            let mut state = self.state();
            let was_enabled = state.settings.oauth_fallback_enabled;
            change(&mut state.settings);
            (state.settings.clone(), !was_enabled && state.settings.oauth_fallback_enabled)
        };
        let _ = settings.save();
        let _ = self.sender.send(Wake::SettingsChanged);
        if enabled_fallback {
            self.refresh(false);
        }
    }
}

impl Drop for UsageModel {
    fn drop(&mut self) {
        let _ = self.sender.send(Wake::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(state: Arc<Mutex<State>>, receiver: Receiver<Wake>, repaint: Arc<dyn Fn() + Send + Sync>) {
    let client = OAuthUsageClient::new();
    // Set while a poll is in flight so a burst of wake-ups collapses into one
    // request rather than queueing several.
    let busy = AtomicBool::new(false);

    loop {
        let interval = {
            let state = state.lock().unwrap_or_else(|p| p.into_inner());
            Duration::from_secs(state.settings.poll_interval_seconds.max(60))
        };

        let wake = match receiver.recv_timeout(interval) {
            Ok(wake) => wake,
            Err(RecvTimeoutError::Timeout) => Wake::Tick,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        match wake {
            Wake::Stop => return,
            // Only re-arm the timer; the loop head re-reads the interval.
            Wake::SettingsChanged => continue,
            Wake::CacheChanged => {
                if load_cache_if_newer(&state) {
                    repaint();
                }
            }
            Wake::Tick => {
                if !busy.swap(true, Ordering::SeqCst) {
                    refresh(&state, &client, false);
                    busy.store(false, Ordering::SeqCst);
                    repaint();
                }
            }
            Wake::Refresh { force } => {
                if !busy.swap(true, Ordering::SeqCst) {
                    refresh(&state, &client, force);
                    busy.store(false, Ordering::SeqCst);
                    repaint();
                }
            }
        }
    }
}

/// File-watch callback. Only adopt a cache write that is actually newer than
/// what we're showing, so a stale file can't clobber a fresh API result.
fn load_cache_if_newer(state: &Arc<Mutex<State>>) -> bool {
    let Some((cached, context)) = status_line_cache::read() else { return false };
    let mut state = state.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(current) = &state.snapshot {
        if cached.captured_at <= current.captured_at {
            return false;
        }
    }
    state.adopt(cached, Some(context));
    state.last_error = None;
    true
}

fn refresh(state: &Arc<Mutex<State>>, client: &OAuthUsageClient, force: bool) {
    let cached = status_line_cache::read();
    let now = Utc::now();

    let decision = {
        let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
        let cached_age = cached.as_ref().map(|(snapshot, _)| snapshot.age_from(now));
        if let Some((snapshot, context)) = cached {
            guard.adopt(snapshot, Some(context));
        }

        let held_age = guard.snapshot.as_ref().map(|snapshot| snapshot.age_from(now));
        let backoff_active = matches!(guard.backoff_until, Some(until) if now < until);
        let decision = plan(
            cached_age,
            held_age,
            force,
            guard.settings.oauth_fallback_enabled,
            backoff_active,
        );

        match decision {
            RefreshPlan::CacheIsFresh | RefreshPlan::CacheOnly | RefreshPlan::HeldRecently => {
                guard.last_error = None;
            }
            RefreshPlan::NoSource => guard.last_error = Some(UsageError::NoData),
            RefreshPlan::BackedOff => {
                guard.last_error = Some(UsageError::RateLimited { retry_at: guard.backoff_until })
            }
            RefreshPlan::Poll => guard.is_refreshing = true,
        }
        decision
    };

    if decision != RefreshPlan::Poll {
        return;
    }

    let result = client.fetch();

    let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
    guard.is_refreshing = false;
    match result {
        Ok(fresh) => {
            guard.adopt(fresh, None);
            guard.last_error = None;
            guard.backoff_until = None;
            guard.consecutive_rate_limits = 0;
        }
        Err(Failure::Credentials(error)) => guard.last_error = Some(error),
        Err(Failure::Transport(UsageError::RateLimited { retry_at })) => {
            guard.consecutive_rate_limits += 1;
            let until = backoff_deadline(guard.consecutive_rate_limits, retry_at, Utc::now());
            guard.backoff_until = Some(until);
            guard.last_error = Some(UsageError::RateLimited { retry_at: Some(until) });
        }
        Err(Failure::Transport(error)) => guard.last_error = Some(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{UsageSource, UsageWindow, UsageWindowKind};
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap()
    }

    #[test]
    fn a_fresh_status_line_cache_never_costs_a_network_call() {
        // The whole point of the app: while a session is open the cache is
        // rewritten every turn, so this is the branch that runs almost always.
        assert_eq!(plan(Some(5.0), Some(5.0), false, true, false), RefreshPlan::CacheIsFresh);
        assert_eq!(plan(Some(119.0), Some(119.0), false, true, false), RefreshPlan::CacheIsFresh);
    }

    #[test]
    fn a_stale_cache_is_what_reaches_for_the_network() {
        assert_eq!(plan(Some(121.0), Some(121.0), false, true, false), RefreshPlan::Poll);
        assert_eq!(plan(None, None, false, true, false), RefreshPlan::Poll);
    }

    #[test]
    fn a_forced_refresh_ignores_freshness_and_backoff_alike() {
        assert_eq!(plan(Some(5.0), Some(5.0), true, true, false), RefreshPlan::Poll);
        assert_eq!(plan(Some(5.0), Some(5.0), true, true, true), RefreshPlan::Poll);
    }

    #[test]
    fn with_the_fallback_off_we_show_the_cache_or_say_there_is_nothing() {
        assert_eq!(plan(Some(600.0), Some(600.0), false, false, false), RefreshPlan::CacheOnly);
        assert_eq!(plan(None, None, false, false, false), RefreshPlan::NoSource);
        // Even forced: turning the fallback off means no network call, period.
        assert_eq!(plan(None, None, true, false, false), RefreshPlan::NoSource);
    }

    #[test]
    fn something_recent_we_already_hold_is_good_enough_to_skip_a_poll() {
        // Cache is gone or stale, but the last OAuth result is 30s old. Opening
        // the popover repeatedly must not turn into a request each time.
        assert_eq!(plan(None, Some(30.0), false, true, false), RefreshPlan::HeldRecently);
        assert_eq!(plan(Some(600.0), Some(30.0), false, true, false), RefreshPlan::HeldRecently);
    }

    #[test]
    fn backoff_suppresses_polling_until_it_elapses() {
        assert_eq!(plan(None, None, false, true, true), RefreshPlan::BackedOff);
        assert_eq!(plan(None, Some(600.0), false, true, true), RefreshPlan::BackedOff);
    }

    #[test]
    fn backoff_doubles_from_one_minute_and_stops_at_the_ceiling() {
        let now = at(0);
        let after = |strikes| (backoff_deadline(strikes, None, now) - now).num_seconds();
        assert_eq!(after(1), 60);
        assert_eq!(after(2), 120);
        assert_eq!(after(3), 240);
        assert_eq!(after(6), 1_800);
        assert_eq!(after(20), 1_800, "clamped, and no overflow on a long outage");
    }

    #[test]
    fn a_retry_after_further_out_wins_but_one_closer_in_cannot_undercut() {
        let now = at(0);
        // The server knows when the window really reopens.
        assert_eq!(backoff_deadline(1, Some(at(600)), now), at(600));
        // "try again in 5 seconds", said for the third time, is not credible.
        assert_eq!(backoff_deadline(1, Some(at(5)), now), at(60));
    }

    #[test]
    fn a_snapshot_older_than_what_we_hold_is_not_adopted() {
        let mut state = State::default();
        let snapshot = |seconds| {
            UsageSnapshot::new(
                vec![crate::snapshot::UsageEntry::new(
                    UsageWindowKind::FiveHour,
                    UsageWindow::new(50.0, None),
                )],
                at(seconds),
                UsageSource::StatusLine,
            )
        };

        state.adopt(snapshot(100), None);
        state.adopt(snapshot(50), None);
        assert_eq!(state.snapshot.as_ref().unwrap().captured_at, at(100));
    }

    #[test]
    fn the_plan_badge_survives_a_status_line_write_that_does_not_carry_one() {
        let mut state = State::default();
        let entries = vec![crate::snapshot::UsageEntry::new(
            UsageWindowKind::FiveHour,
            UsageWindow::new(50.0, None),
        )];

        let mut from_api = UsageSnapshot::new(entries.clone(), at(0), UsageSource::OAuth);
        from_api.plan = Some("Max (5x)".to_string());
        state.adopt(from_api, None);

        // The status line has no idea what plan you are on.
        state.adopt(UsageSnapshot::new(entries, at(60), UsageSource::StatusLine), None);
        assert_eq!(state.snapshot.as_ref().unwrap().plan.as_deref(), Some("Max (5x)"));
    }

    #[test]
    fn the_empty_state_does_not_also_print_an_error_saying_the_same_thing() {
        let mut state = State::default();
        state.last_error = Some(UsageError::NoData);
        assert_eq!(state.visible_error(), None);

        // Once there is something to show, a later NoData is worth surfacing.
        state.snapshot = Some(UsageSnapshot::new(vec![], at(0), UsageSource::OAuth));
        assert_eq!(state.visible_error(), Some(&UsageError::NoData));
    }

    #[test]
    fn staleness_is_two_missed_refreshes_not_a_fixed_interval() {
        let mut state = State::default();
        assert!(state.is_stale(), "nothing at all is as stale as it gets");

        state.settings.poll_interval_seconds = 60;
        state.snapshot =
            Some(UsageSnapshot::new(vec![], Utc::now(), UsageSource::StatusLine));
        assert!(!state.is_stale());

        state.snapshot = Some(UsageSnapshot::new(
            vec![],
            Utc::now() - chrono::Duration::seconds(200),
            UsageSource::StatusLine,
        ));
        assert!(state.is_stale());
    }
}
