//! Port of `menubar/Sources/ClaudeQuotaBar/Settings.swift`.
//!
//! macOS persists these in `UserDefaults`; there is no equivalent on Windows
//! that isn't the registry, and the registry is the wrong place for something a
//! user might want to read, copy between machines, or delete. A JSON file under
//! `%APPDATA%\ClaudeQuota` it is.

use serde::{Deserialize, Serialize};

use crate::snapshot::{UsageEntry, UsageSnapshot, UsageWindowKind};

/// What the tray icon itself shows. The popover always lists every window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BarDisplayMode {
    Session,
    Weekly,
    Worst,
}

impl BarDisplayMode {
    pub const ALL: [BarDisplayMode; 3] = [
        BarDisplayMode::Session,
        BarDisplayMode::Weekly,
        BarDisplayMode::Worst,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BarDisplayMode::Session => "Session (5h)",
            BarDisplayMode::Weekly => "Weekly (7d)",
            BarDisplayMode::Worst => "Whichever is highest",
        }
    }

    /// Falls back to the worst window when the preferred one isn't reported --
    /// better to show something true than an empty indicator.
    pub fn entry(self, snapshot: &UsageSnapshot) -> Option<UsageEntry> {
        let preferred = match self {
            BarDisplayMode::Session => Some(UsageWindowKind::FiveHour),
            BarDisplayMode::Weekly => Some(UsageWindowKind::SevenDay),
            BarDisplayMode::Worst => None,
        };
        match preferred {
            None => snapshot.peak(),
            Some(kind) => match snapshot.window(kind) {
                Some(window) => Some(UsageEntry::new(kind, window.clone())),
                None => snapshot.peak(),
            },
        }
    }
}

/// What shape the tray indicator takes.
///
/// `Number` has no macOS counterpart, and exists because of a real platform
/// difference: a `MenuBarExtra` label can carry an image *and* text, so the
/// macOS app draws the ring and writes "23%" beside it. A Windows tray icon is
/// a bare 16x16 bitmap with no label at all, so the percentage has to go
/// *inside* the icon or nowhere. Upstream's `showPercentageText` toggle becomes
/// this third style; the tooltip carries the numbers either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndicatorStyle {
    Ring,
    Bar,
    Number,
}

impl IndicatorStyle {
    pub const ALL: [IndicatorStyle; 3] = [
        IndicatorStyle::Ring,
        IndicatorStyle::Bar,
        IndicatorStyle::Number,
    ];

    pub fn label(self) -> &'static str {
        match self {
            IndicatorStyle::Ring => "Ring",
            IndicatorStyle::Bar => "Bar",
            IndicatorStyle::Number => "Number",
        }
    }
}

/// How a percentage picks its colour.
///
/// Two honest positions, not a spectrum: match Claude's own usage panel, or
/// let the fill carry severity. The tray icon and the popover always agree --
/// an icon that says orange while the popover says blue is worse than either
/// on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorTheme {
    /// One flat blue at every level, like Claude's usage screen.
    Claude,
    /// Green below 50%, yellow to 80%, orange to 95%, red above.
    Usage,
}

impl ColorTheme {
    pub const ALL: [ColorTheme; 2] = [ColorTheme::Claude, ColorTheme::Usage];

    pub fn label(self) -> &'static str {
        match self {
            ColorTheme::Claude => "Claude blue",
            ColorTheme::Usage => "Usage ramp",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            ColorTheme::Claude => {
                "Matches Claude's usage screen. Colour never signals how full you are."
            }
            ColorTheme::Usage => "Green under 50%, yellow to 80%, orange to 95%, red above.",
        }
    }
}

pub mod poll_interval {
    /// Options offered in Settings, in seconds.
    pub const CHOICES: [u64; 4] = [60, 300, 600, 1_800];
    pub const DEFAULT: u64 = 60;

    pub fn label(seconds: u64) -> String {
        if seconds < 3_600 {
            format!("{} min", seconds / 60)
        } else {
            format!("{} hr", seconds / 3_600)
        }
    }
}

pub mod defaults {
    /// A status line snapshot older than this is treated as stale, which is what
    /// triggers an OAuth poll. Sized so an active session (which re-renders on
    /// every turn) essentially never triggers a network call.
    pub const CACHE_STALE_AFTER: f64 = 120.0;

    /// First backoff after a 429, doubling per consecutive rate limit. Fixed
    /// rather than derived from the poll interval, which at the 30-minute
    /// setting would land on the ceiling from the very first 429 and make the
    /// exponential meaningless.
    pub const BACKOFF_BASE: f64 = 60.0;

    /// Backoff ceiling after repeated 429s from the usage endpoint.
    pub const MAX_BACKOFF: f64 = 1_800.0;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub poll_interval_seconds: u64,
    pub bar_display_mode: BarDisplayMode,
    pub oauth_fallback_enabled: bool,
    pub indicator_style: IndicatorStyle,
    pub color_theme: ColorTheme,
    /// The status line command that was configured before we took it over.
    ///
    /// macOS threads this through a `CLAUDE_QUOTA_CHAIN=<cmd>` prefix on the
    /// command string itself. That is bash-only syntax, and Claude Code on
    /// Windows runs the status line through PowerShell whenever Git Bash is
    /// absent -- where the prefix is a parse error. Keeping it here instead
    /// means the command string stays a bare, unquoted, shell-agnostic path.
    pub chain: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_interval_seconds: poll_interval::DEFAULT,
            bar_display_mode: BarDisplayMode::Session,
            oauth_fallback_enabled: true,
            indicator_style: IndicatorStyle::Ring,
            color_theme: ColorTheme::Claude,
            chain: None,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&crate::paths::settings_file())
    }

    /// A settings file we can't read or parse is not worth refusing to start
    /// over -- fall back to the defaults, exactly as `UserDefaults.register`
    /// does on macOS.
    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&crate::paths::settings_file())
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(path, json + "\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{UsageSource, UsageWindow};
    use chrono::{TimeZone, Utc};

    fn snapshot(windows: Vec<UsageEntry>) -> UsageSnapshot {
        UsageSnapshot::new(
            windows,
            Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            UsageSource::OAuth,
        )
    }

    fn entry(kind: UsageWindowKind, percent: f64) -> UsageEntry {
        UsageEntry::new(kind, UsageWindow::new(percent, None))
    }

    #[test]
    fn each_display_mode_picks_its_own_window() {
        let snapshot = snapshot(vec![
            entry(UsageWindowKind::FiveHour, 23.0),
            entry(UsageWindowKind::SevenDay, 91.0),
        ]);
        assert_eq!(
            BarDisplayMode::Session.entry(&snapshot).unwrap().kind,
            UsageWindowKind::FiveHour
        );
        assert_eq!(
            BarDisplayMode::Weekly.entry(&snapshot).unwrap().kind,
            UsageWindowKind::SevenDay
        );
        assert_eq!(
            BarDisplayMode::Worst.entry(&snapshot).unwrap().kind,
            UsageWindowKind::SevenDay
        );
    }

    #[test]
    fn a_mode_whose_window_is_missing_shows_the_worst_rather_than_nothing() {
        let weekly_only = snapshot(vec![entry(UsageWindowKind::SevenDay, 41.0)]);
        assert_eq!(
            BarDisplayMode::Session.entry(&weekly_only).unwrap().kind,
            UsageWindowKind::SevenDay
        );

        let session_only = snapshot(vec![entry(UsageWindowKind::FiveHour, 23.0)]);
        assert_eq!(
            BarDisplayMode::Weekly.entry(&session_only).unwrap().kind,
            UsageWindowKind::FiveHour
        );
    }

    #[test]
    fn an_empty_snapshot_selects_no_window_in_any_mode() {
        let empty = snapshot(vec![]);
        for mode in BarDisplayMode::ALL {
            assert!(mode.entry(&empty).is_none(), "{mode:?}");
        }
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");

        let settings = Settings {
            poll_interval_seconds: 600,
            bar_display_mode: BarDisplayMode::Worst,
            oauth_fallback_enabled: false,
            indicator_style: IndicatorStyle::Number,
            color_theme: ColorTheme::Usage,
            chain: Some("powershell -File C:/me/status.ps1".to_string()),
        };
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);
    }

    #[test]
    fn a_missing_or_corrupt_settings_file_falls_back_to_the_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("nope.json");
        assert_eq!(Settings::load_from(&missing), Settings::default());

        let corrupt = directory.path().join("corrupt.json");
        std::fs::write(&corrupt, "{ not json").unwrap();
        assert_eq!(Settings::load_from(&corrupt), Settings::default());
    }

    #[test]
    fn a_settings_file_from_an_older_build_keeps_its_known_keys() {
        // `#[serde(default)]` on the struct is what makes a partial file usable
        // instead of throwing the user's whole configuration away.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"colorTheme": "usage"}"#).unwrap();

        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.color_theme, ColorTheme::Usage);
        assert_eq!(loaded.poll_interval_seconds, poll_interval::DEFAULT);
        assert!(loaded.oauth_fallback_enabled);
    }

    #[test]
    fn poll_interval_labels_read_the_way_the_settings_screen_needs() {
        assert_eq!(poll_interval::label(60), "1 min");
        assert_eq!(poll_interval::label(1_800), "30 min");
        assert_eq!(poll_interval::label(3_600), "1 hr");
    }
}
