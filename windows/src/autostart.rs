//! Launch at login.
//!
//! The macOS app registers with `SMAppService`. The Windows equivalent that
//! needs no installer, no admin rights, and no scheduled task is a value under
//! the per-user Run key, which is also the one place users know to look when
//! they wonder what starts with their machine.

use std::path::Path;

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "ClaudeQuota";

fn run_key(access: u32) -> std::io::Result<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, access)
}

pub fn is_enabled() -> bool {
    run_key(KEY_READ)
        .and_then(|key| key.get_value::<String, _>(VALUE))
        .map(|value| !value.is_empty())
        .unwrap_or(false)
}

/// The stored command is quoted, unlike the status line one: this is read by
/// the shell's own launcher, which understands quotes and needs them for a path
/// with a space in it.
pub fn enable(exe: &Path) -> std::io::Result<()> {
    let key = run_key(KEY_WRITE)?;
    key.set_value(VALUE, &format!("\"{}\"", exe.display()))
}

pub fn disable() -> std::io::Result<()> {
    match run_key(KEY_WRITE)?.delete_value(VALUE) {
        Ok(()) => Ok(()),
        // Already gone is the state the caller asked for.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn set(enabled: bool, exe: &Path) -> std::io::Result<()> {
    if enabled {
        enable(exe)
    } else {
        disable()
    }
}
