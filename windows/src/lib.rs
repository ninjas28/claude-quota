//! Claude usage in the Windows notification area.
//!
//! A port of the macOS menu bar app in `menubar/`. The data layer (snapshot,
//! oauth, credentials, status_line_cache, model) is a faithful translation and
//! is covered by tests that mirror the Swift suite; the UI layer is native to
//! Windows and shares only the colour rules and the popover layout.

pub mod app;
pub mod bridge;
pub mod credentials;
pub mod gauge;
pub mod log;
pub mod model;
pub mod oauth;
pub mod paths;
pub mod platform;
pub mod popover;
pub mod relative_time;
pub mod settings;
pub mod settings_window;
pub mod snapshot;
pub mod status_line_cache;
pub mod statusline_install;
pub mod tray;

#[cfg(windows)]
pub mod autostart;
