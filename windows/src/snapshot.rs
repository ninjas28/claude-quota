//! The rolling windows Anthropic reports for a Claude.ai subscription.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/UsageSnapshot.swift`.

use chrono::{DateTime, Utc};

use crate::relative_time;

/// `FiveHour` is the "session" window most people care about minute to minute;
/// the weekly ones are the ceilings. Model-scoped weekly windows only exist on
/// some plans, so every window is optional at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UsageWindowKind {
    FiveHour,
    SevenDay,
    /// A weekly ceiling that applies to one model rather than the whole plan.
    /// The model it covers arrives at runtime, in `UsageWindow::scope_label`.
    WeeklyScoped,
}

impl UsageWindowKind {
    /// Order used when rendering the popover.
    pub const DISPLAY_ORDER: [UsageWindowKind; 3] = [
        UsageWindowKind::FiveHour,
        UsageWindowKind::SevenDay,
        UsageWindowKind::WeeklyScoped,
    ];

    pub fn key(self) -> &'static str {
        match self {
            UsageWindowKind::FiveHour => "fiveHour",
            UsageWindowKind::SevenDay => "sevenDay",
            UsageWindowKind::WeeklyScoped => "weeklyScoped",
        }
    }

    /// Row labels, phrased the way Claude's usage screen phrases them. The
    /// weekly rows don't repeat "Weekly", the section header carries that.
    pub fn label(self) -> &'static str {
        match self {
            UsageWindowKind::FiveHour => "Current session",
            UsageWindowKind::SevenDay => "All models",
            UsageWindowKind::WeeklyScoped => "Scoped",
        }
    }

    /// Whether this window belongs under the "Weekly limits" heading.
    pub fn is_weekly(self) -> bool {
        self != UsageWindowKind::FiveHour
    }

    fn rank(self) -> usize {
        Self::DISPLAY_ORDER
            .iter()
            .position(|kind| *kind == self)
            .unwrap_or(usize::MAX)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    /// 0...100. Both sources report this as a percentage, not a 0-1 fraction.
    /// (The legacy CLI's response headers use 0-1 -- different source, different
    /// scale. The `oauth` tests pin this down.)
    pub used_percentage: f64,
    pub resets_at: Option<DateTime<Utc>>,
    /// What this window is scoped to, when it isn't the whole plan -- e.g. the
    /// display name of a single model. None for plan-wide windows.
    pub scope_label: Option<String>,
}

impl UsageWindow {
    pub fn new(used_percentage: f64, resets_at: Option<DateTime<Utc>>) -> Self {
        Self {
            used_percentage,
            resets_at,
            scope_label: None,
        }
    }

    pub fn scoped(
        used_percentage: f64,
        resets_at: Option<DateTime<Utc>>,
        scope_label: Option<String>,
    ) -> Self {
        Self {
            used_percentage,
            resets_at,
            scope_label,
        }
    }

    pub fn clamped_percentage(&self) -> f64 {
        self.used_percentage.clamp(0.0, 100.0)
    }
}

/// Where a snapshot came from. Both sources report the same numbers; the
/// distinction matters only for explaining staleness to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    /// Handed to us locally by Claude Code's status line. Free, no network call.
    StatusLine,
    /// Polled from the OAuth usage endpoint. Free of quota, but a network call.
    OAuth,
}

impl UsageSource {
    pub fn label(self) -> &'static str {
        match self {
            UsageSource::StatusLine => "Claude Code session",
            UsageSource::OAuth => "usage API",
        }
    }
}

/// One window paired with its kind.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageEntry {
    pub kind: UsageWindowKind,
    pub window: UsageWindow,
}

impl UsageEntry {
    pub fn new(kind: UsageWindowKind, window: UsageWindow) -> Self {
        Self { kind, window }
    }

    /// Scope is part of the identity: an account can report several scoped
    /// weekly windows, which share a `kind` but must not share a list id.
    pub fn id(&self) -> String {
        match &self.window.scope_label {
            Some(scope) => format!("{}-{}", self.kind.key(), scope),
            None => self.kind.key().to_string(),
        }
    }

    /// A scoped window is named after the model it covers ("Fable"); everything
    /// else uses its kind's label.
    pub fn label(&self) -> &str {
        match self.window.scope_label.as_deref() {
            Some(scope) => scope,
            None => self.kind.label(),
        }
    }

    /// The line under the label. Claude's screen shows the countdown, or a
    /// "not used yet" note for a window still sitting at zero.
    pub fn subtitle(&self, now: DateTime<Utc>) -> Option<String> {
        if self.window.clamped_percentage() == 0.0 {
            let what = self.window.scope_label.as_deref().unwrap_or("this");
            return Some(format!("You haven{}t used {} yet", '\u{2019}', what));
        }
        let resets_at = self.window.resets_at?;
        if resets_at <= now {
            return Some("Reset".to_string());
        }
        Some(format!(
            "Resets in {}",
            relative_time::long_countdown(resets_at, now)
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageSnapshot {
    /// An ordered list rather than a map keyed by kind: the usage endpoint can
    /// report more than one `weekly_scoped` window (one per model), and those
    /// collide on `kind`.
    pub windows: Vec<UsageEntry>,
    /// Whether the account has extra/overage usage enabled. Absent when unknown.
    pub extra_usage_enabled: Option<bool>,
    pub captured_at: DateTime<Utc>,
    pub source: UsageSource,
    /// Plan badge for the header ("Max (5x)"). Only the OAuth path knows it --
    /// it comes off the stored credential, not the usage payload.
    pub plan: Option<String>,
}

impl UsageSnapshot {
    pub fn new(windows: Vec<UsageEntry>, captured_at: DateTime<Utc>, source: UsageSource) -> Self {
        Self {
            windows,
            extra_usage_enabled: None,
            captured_at,
            source,
            plan: None,
        }
    }

    pub fn window(&self, kind: UsageWindowKind) -> Option<&UsageWindow> {
        self.windows
            .iter()
            .find(|entry| entry.kind == kind)
            .map(|entry| &entry.window)
    }

    pub fn age_from(&self, now: DateTime<Utc>) -> f64 {
        ((now - self.captured_at).num_milliseconds() as f64 / 1000.0).max(0.0)
    }

    pub fn age(&self) -> f64 {
        self.age_from(Utc::now())
    }

    /// Windows that actually have data, in display order. Sorted on the index
    /// into `DISPLAY_ORDER` and the original position, so several windows of the
    /// same kind keep the order the server sent them in.
    pub fn present_windows(&self) -> Vec<UsageEntry> {
        let mut indexed: Vec<(usize, &UsageEntry)> = self.windows.iter().enumerate().collect();
        indexed.sort_by(|left, right| {
            left.1
                .kind
                .rank()
                .cmp(&right.1.kind.rank())
                .then(left.0.cmp(&right.0))
        });
        indexed
            .into_iter()
            .map(|(_, entry)| entry.clone())
            .collect()
    }

    /// The two groups Claude's usage screen splits windows into.
    pub fn session_windows(&self) -> Vec<UsageEntry> {
        self.present_windows()
            .into_iter()
            .filter(|e| !e.kind.is_weekly())
            .collect()
    }

    pub fn weekly_windows(&self) -> Vec<UsageEntry> {
        self.present_windows()
            .into_iter()
            .filter(|e| e.kind.is_weekly())
            .collect()
    }

    /// The highest utilization across every reported window -- what the tray icon
    /// shows in "worst window" mode, since that is the one that will bite first.
    pub fn peak(&self) -> Option<UsageEntry> {
        // `max_by` returns the *last* maximum, matching Swift's `max(by:)`.
        self.present_windows().into_iter().max_by(|left, right| {
            left.window
                .clamped_percentage()
                .partial_cmp(&right.window.clamped_percentage())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

/// A user-facing problem that stopped us from refreshing.
#[derive(Debug, Clone, PartialEq)]
pub enum UsageError {
    NoCredentials,
    CredentialsExpired,
    RateLimited { retry_at: Option<DateTime<Utc>> },
    NotSubscribed,
    Network(String),
    NoData,
}

impl UsageError {
    pub fn title(&self) -> &'static str {
        match self {
            UsageError::NoCredentials => "Claude Code not logged in",
            UsageError::CredentialsExpired => "Login expired",
            UsageError::RateLimited { .. } => "Usage API rate limited",
            UsageError::NotSubscribed => "No subscription usage",
            UsageError::Network(_) => "Couldn't reach the usage API",
            UsageError::NoData => "No usage data yet",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            UsageError::NoCredentials => {
                "Run `claude` once and sign in, or turn off the usage API fallback in Settings."
                    .to_string()
            }
            UsageError::CredentialsExpired => {
                "Open Claude Code to refresh the login. Claude Quota never refreshes tokens itself."
                    .to_string()
            }
            UsageError::RateLimited { retry_at } => match retry_at {
                None => "Backing off before trying again.".to_string(),
                Some(retry_at) => {
                    format!("Backing off until {}.", relative_time::clock(*retry_at))
                }
            },
            UsageError::NotSubscribed => {
                "Rate limit windows are only reported for Claude Pro and Max accounts.".to_string()
            }
            UsageError::Network(message) => message.clone(),
            UsageError::NoData => {
                "Start a Claude Code session, or enable the usage API fallback in Settings."
                    .to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap()
    }

    fn entry(kind: UsageWindowKind, percent: f64) -> UsageEntry {
        UsageEntry::new(kind, UsageWindow::new(percent, Some(at(3600))))
    }

    #[test]
    fn percentages_clamp_to_the_reportable_range() {
        assert_eq!(UsageWindow::new(-4.0, None).clamped_percentage(), 0.0);
        assert_eq!(UsageWindow::new(140.0, None).clamped_percentage(), 100.0);
        assert_eq!(UsageWindow::new(62.5, None).clamped_percentage(), 62.5);
    }

    #[test]
    fn windows_render_in_claudes_order_regardless_of_arrival_order() {
        let snapshot = UsageSnapshot::new(
            vec![
                entry(UsageWindowKind::SevenDay, 41.0),
                entry(UsageWindowKind::WeeklyScoped, 10.0),
                entry(UsageWindowKind::FiveHour, 23.0),
            ],
            at(0),
            UsageSource::OAuth,
        );
        let kinds: Vec<_> = snapshot.present_windows().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                UsageWindowKind::FiveHour,
                UsageWindowKind::SevenDay,
                UsageWindowKind::WeeklyScoped
            ]
        );
    }

    #[test]
    fn several_scoped_windows_keep_the_order_the_server_sent() {
        let scoped = |name: &str, percent: f64| {
            UsageEntry::new(
                UsageWindowKind::WeeklyScoped,
                UsageWindow::scoped(percent, None, Some(name.to_string())),
            )
        };
        let snapshot = UsageSnapshot::new(
            vec![scoped("Opus", 12.0), scoped("Fable", 8.0)],
            at(0),
            UsageSource::OAuth,
        );
        let labels: Vec<String> = snapshot
            .present_windows()
            .iter()
            .map(|e| e.label().to_string())
            .collect();
        assert_eq!(labels, vec!["Opus", "Fable"]);
    }

    #[test]
    fn scoped_windows_do_not_share_an_id() {
        let scoped = |name: &str| {
            UsageEntry::new(
                UsageWindowKind::WeeklyScoped,
                UsageWindow::scoped(1.0, None, Some(name.to_string())),
            )
        };
        assert_ne!(scoped("Opus").id(), scoped("Fable").id());
        assert_eq!(entry(UsageWindowKind::FiveHour, 1.0).id(), "fiveHour");
    }

    #[test]
    fn peak_is_the_window_that_will_bite_first() {
        let snapshot = UsageSnapshot::new(
            vec![
                entry(UsageWindowKind::FiveHour, 23.0),
                entry(UsageWindowKind::SevenDay, 91.0),
                entry(UsageWindowKind::WeeklyScoped, 44.0),
            ],
            at(0),
            UsageSource::OAuth,
        );
        assert_eq!(snapshot.peak().unwrap().kind, UsageWindowKind::SevenDay);
    }

    #[test]
    fn peak_clamps_before_comparing_so_an_overage_cannot_win_on_raw_value() {
        let snapshot = UsageSnapshot::new(
            vec![
                UsageEntry::new(UsageWindowKind::FiveHour, UsageWindow::new(140.0, None)),
                UsageEntry::new(UsageWindowKind::SevenDay, UsageWindow::new(100.0, None)),
            ],
            at(0),
            UsageSource::OAuth,
        );
        // Both clamp to 100; `max_by` keeps the last, matching Swift's `max(by:)`.
        assert_eq!(snapshot.peak().unwrap().kind, UsageWindowKind::SevenDay);
    }

    #[test]
    fn weekly_grouping_matches_claudes_two_sections() {
        let snapshot = UsageSnapshot::new(
            vec![
                entry(UsageWindowKind::FiveHour, 23.0),
                entry(UsageWindowKind::SevenDay, 41.0),
                entry(UsageWindowKind::WeeklyScoped, 10.0),
            ],
            at(0),
            UsageSource::OAuth,
        );
        assert_eq!(snapshot.session_windows().len(), 1);
        assert_eq!(snapshot.weekly_windows().len(), 2);
    }

    #[test]
    fn an_untouched_window_says_so_instead_of_counting_down() {
        let untouched = UsageEntry::new(
            UsageWindowKind::WeeklyScoped,
            UsageWindow::scoped(0.0, Some(at(3600)), Some("Fable".to_string())),
        );
        assert!(untouched
            .subtitle(at(0))
            .unwrap()
            .ends_with("used Fable yet"));

        let plain = UsageEntry::new(
            UsageWindowKind::FiveHour,
            UsageWindow::new(0.0, Some(at(60))),
        );
        assert!(plain.subtitle(at(0)).unwrap().ends_with("used this yet"));
    }

    #[test]
    fn a_used_window_counts_down_and_then_reads_as_reset() {
        let used = entry(UsageWindowKind::FiveHour, 23.0);
        assert_eq!(used.subtitle(at(0)).unwrap(), "Resets in 1 hr");
        assert_eq!(used.subtitle(at(7200)).unwrap(), "Reset");
    }

    #[test]
    fn a_used_window_with_no_reset_time_has_no_subtitle() {
        let used = UsageEntry::new(UsageWindowKind::FiveHour, UsageWindow::new(23.0, None));
        assert_eq!(used.subtitle(at(0)), None);
    }

    #[test]
    fn age_never_goes_negative_on_a_clock_that_stepped_backwards() {
        let snapshot = UsageSnapshot::new(vec![], at(60), UsageSource::StatusLine);
        assert_eq!(snapshot.age_from(at(0)), 0.0);
        assert_eq!(snapshot.age_from(at(120)), 60.0);
    }
}
