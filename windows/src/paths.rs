//! Every path the app touches, in one place.
//!
//! macOS puts settings in `UserDefaults` and reads the credential from the
//! Keychain; on Windows both are files, so it is worth being explicit about
//! where they live and which environment variables can move them.

use std::path::PathBuf;

/// `~/.claude`, where Claude Code keeps its settings, credential, and where the
/// status line bridge writes the quota cache.
pub fn claude_home() -> PathBuf {
    if let Ok(override_path) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    home().join(".claude")
}

pub fn claude_settings() -> PathBuf {
    claude_home().join("settings.json")
}

/// `%APPDATA%\ClaudeQuota` -- our own settings, the `UserDefaults` equivalent.
pub fn app_data() -> PathBuf {
    if let Ok(override_path) = std::env::var("CLAUDE_QUOTA_HOME") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| home().join("AppData").join("Roaming"))
        .join("ClaudeQuota")
}

pub fn settings_file() -> PathBuf {
    app_data().join("settings.json")
}

/// Where `install.ps1` puts the executable. Deliberately short and, on a normal
/// account, free of spaces -- the status line command string cannot be quoted
/// (see `statusline_install`), so the path has to survive unquoted.
pub fn install_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| home().join("AppData").join("Local"))
        .join("ClaudeQuota")
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}
