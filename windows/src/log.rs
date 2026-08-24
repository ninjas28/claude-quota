//! An opt-in debug log.
//!
//! The tray app is a windows-subsystem binary with no console, so there is
//! nowhere for a diagnostic to go by default. Set `CLAUDE_QUOTA_LOG` to a file
//! path and every event below is appended to it. Off unless that is set, and it
//! never records anything from the credential.

use std::io::Write;

pub fn enabled() -> bool {
    path().is_some()
}

fn path() -> Option<String> {
    std::env::var("CLAUDE_QUOTA_LOG").ok().filter(|value| !value.is_empty())
}

pub fn write(message: &str) {
    let Some(path) = path() else { return };
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let stamp = chrono::Local::now().format("%H:%M:%S%.3f");
    let _ = writeln!(file, "{stamp} {message}");
}

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if $crate::log::enabled() {
            $crate::log::write(&format!($($arg)*));
        }
    };
}
