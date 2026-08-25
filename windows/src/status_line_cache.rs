//! Reads the cache file written by the status line bridge.
//!
//! Port of `menubar/Sources/ClaudeQuotaBar/StatusLineCache.swift`.
//!
//! Claude Code hands `rate_limits` to the status line command on stdin with no
//! network call of its own, so this path is free, instant, and refreshes on
//! every render while you're working. It is the app's primary source; the OAuth
//! poll only covers the gap when no session is running.
//!
//! The file format is shared with the macOS app and with
//! `statusline/claude-quota-statusline.py`, so either bridge can feed either
//! app. `tests::reads_exactly_what_the_python_bridge_writes` is what keeps that
//! promise honest.

use chrono::{DateTime, Utc};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use std::path::PathBuf;

use crate::oauth::{decode_date, decode_number};
use crate::snapshot::{UsageEntry, UsageSnapshot, UsageSource, UsageWindow, UsageWindowKind};

/// Extra detail the status line can tell us that the API can't.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    pub model: Option<String>,
    pub session_id: Option<String>,
}

pub fn cache_path() -> PathBuf {
    if let Ok(override_path) = std::env::var("CLAUDE_QUOTA_CACHE") {
        if !override_path.is_empty() {
            return PathBuf::from(shellexpand_tilde(&override_path));
        }
    }
    crate::paths::claude_home().join("quota-bar-cache.json")
}

/// The macOS installer writes `~/...` into `CLAUDE_QUOTA_CACHE`, and nothing on
/// Windows expands that for us.
fn shellexpand_tilde(path: &str) -> String {
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => dirs::home_dir()
            .map(|home| home.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        None => path.to_string(),
    }
}

pub fn read() -> Option<(UsageSnapshot, Context)> {
    read_at(&cache_path())
}

pub fn read_at(path: &std::path::Path) -> Option<(UsageSnapshot, Context)> {
    let data = std::fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&data).ok()?;
    let root = root.as_object()?;

    let windows: Vec<UsageEntry> = [
        (UsageWindowKind::FiveHour, "five_hour"),
        (UsageWindowKind::SevenDay, "seven_day"),
    ]
    .into_iter()
    .filter_map(|(kind, key)| {
        let object = root.get(key)?;
        let used = decode_number(object.get("used_percentage"))?;
        Some(UsageEntry::new(
            kind,
            UsageWindow::new(used, decode_date(object.get("resets_at"))),
        ))
    })
    .collect();

    if windows.is_empty() {
        return None;
    }

    // Fall back to the file's own mtime if the writer didn't stamp it.
    let captured_at = decode_date(root.get("captured_at"))
        .or_else(|| modified_at(path))
        .unwrap_or_else(Utc::now);

    let snapshot = UsageSnapshot::new(windows, captured_at, UsageSource::StatusLine);
    let context = Context {
        model: root
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        session_id: root
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    Some((snapshot, context))
}

fn modified_at(path: &std::path::Path) -> Option<DateTime<Utc>> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified))
}

/// Watches the cache file so an active Claude Code session updates the tray
/// immediately, instead of waiting for the next poll tick.
///
/// The bridge writes atomically (write temp, rename), which replaces the file
/// rather than modifying it -- so we watch the *containing directory*. Watching
/// the file alone sees the first write and nothing after it.
pub struct CacheFileWatcher {
    _watcher: RecommendedWatcher,
}

impl CacheFileWatcher {
    /// `on_change` fires on a notify worker thread, so it must be cheap and
    /// must not touch the UI directly -- the model posts a wake-up and lets its
    /// own thread do the reading.
    pub fn start(on_change: impl Fn() + Send + 'static) -> notify::Result<Self> {
        let directory = cache_path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&directory);

        let target = cache_path();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                // ReadDirectoryChangesW reports every file in `~/.claude`, which is
                // a busy directory -- Claude Code rewrites history, sessions, and
                // caches constantly. Only our file is worth waking the UI for.
                if event.paths.iter().any(|path| path == &target) {
                    on_change();
                }
            })?;
        watcher.watch(&directory, RecursiveMode::NonRecursive)?;
        Ok(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_cache(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("quota-bar-cache.json");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        (directory, path)
    }

    #[test]
    fn reads_exactly_what_the_python_bridge_writes() {
        // Byte-for-byte the payload `write_cache` in
        // statusline/claude-quota-statusline.py produces. If either side ever
        // renames a field, this test is what catches it.
        let (_directory, path) = write_cache(
            r#"{"five_hour": {"used_percentage": 23.5, "resets_at": 1738425600},
                "seven_day": {"used_percentage": 41.2, "resets_at": 1738857600},
                "captured_at": 1700000000.25,
                "model": "Opus 5",
                "session_id": "sess-abc123"}"#,
        );

        let (snapshot, context) = read_at(&path).unwrap();
        assert_eq!(
            snapshot
                .window(UsageWindowKind::FiveHour)
                .unwrap()
                .used_percentage,
            23.5
        );
        assert_eq!(
            snapshot
                .window(UsageWindowKind::SevenDay)
                .unwrap()
                .used_percentage,
            41.2
        );
        assert_eq!(
            snapshot
                .window(UsageWindowKind::FiveHour)
                .unwrap()
                .resets_at
                .unwrap()
                .timestamp(),
            1_738_425_600
        );
        assert_eq!(snapshot.captured_at.timestamp(), 1_700_000_000);
        assert_eq!(snapshot.source, UsageSource::StatusLine);
        assert_eq!(context.model.as_deref(), Some("Opus 5"));
        assert_eq!(context.session_id.as_deref(), Some("sess-abc123"));
    }

    #[test]
    fn one_window_is_enough_to_be_worth_showing() {
        let (_directory, path) = write_cache(r#"{"five_hour": {"used_percentage": 5}}"#);
        let (snapshot, _) = read_at(&path).unwrap();
        assert_eq!(snapshot.windows.len(), 1);
    }

    #[test]
    fn a_cache_with_no_windows_is_not_a_snapshot() {
        let (_directory, path) = write_cache(r#"{"captured_at": 1700000000, "model": "Opus 5"}"#);
        assert!(read_at(&path).is_none());

        let (_directory, path) = write_cache("half-written{");
        assert!(read_at(&path).is_none());
    }

    #[test]
    fn a_missing_cache_file_is_not_an_error() {
        assert!(read_at(std::path::Path::new("no-such-file.json")).is_none());
    }

    #[test]
    fn an_unstamped_cache_falls_back_to_the_files_own_mtime() {
        let (_directory, path) = write_cache(r#"{"five_hour": {"used_percentage": 5}}"#);
        let (snapshot, _) = read_at(&path).unwrap();
        // Written moments ago, so it must read as fresh rather than as 1970.
        assert!(snapshot.age() < 60.0, "age was {}", snapshot.age());
    }

    #[test]
    fn a_window_missing_its_percentage_is_skipped_but_the_others_survive() {
        let (_directory, path) = write_cache(
            r#"{"five_hour": {"resets_at": 1738425600},
                "seven_day": {"used_percentage": 41.2}}"#,
        );
        let (snapshot, _) = read_at(&path).unwrap();
        assert_eq!(snapshot.windows.len(), 1);
        assert_eq!(snapshot.windows[0].kind, UsageWindowKind::SevenDay);
    }
}
