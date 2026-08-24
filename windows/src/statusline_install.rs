//! Wires the status line bridge into `~/.claude/settings.json`.
//!
//! Port of `statusline/install.py`, with the one part that could not be ported:
//! the command string itself.
//!
//! Claude Code on Windows runs `statusLine.command` **through Git Bash when Git
//! Bash is installed, and through PowerShell when it isn't**. A single string
//! has to work under both, which rules out everything the macOS installer does:
//!
//!   * `CLAUDE_QUOTA_CHAIN=<cmd> <script>` -- a bash-only prefix, and a parse
//!     error in PowerShell. The chained command moves into our settings file.
//!   * `shlex.quote(...)` -- POSIX quoting, and worse, a *quoted* executable
//!     path does not execute in PowerShell at all: `"C:/x/app.exe" arg` prints
//!     the string. PowerShell needs `& "C:/x/app.exe"`, and Git Bash then tries
//!     to run a program called `&`. There is no quoting that satisfies both.
//!   * Backslashes -- Git Bash eats them as escapes, so the path silently loses
//!     its separators and the command fails with no visible error.
//!
//! What is left is a bare, unquoted, forward-slash path -- which only works if
//! it has no spaces in it. `install_command` is where that is enforced, falling
//! back to the 8.3 short name when the profile path contains a space.

use std::path::Path;

use serde_json::{json, Value};

use crate::settings::Settings;

/// How we recognise a command as ours, whatever path it was installed at.
const MARKER: &str = "claude-quota";

pub fn is_ours(command: &str) -> bool {
    let lowered = command.to_lowercase();
    lowered.contains(MARKER) && lowered.contains("statusline")
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandError {
    /// The install path contains a space and has no 8.3 short name, so no
    /// command string we could write would survive both shells.
    UnquotablePath(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::UnquotablePath(path) => write!(
                formatter,
                "the install path contains a space and Windows reports no short name for it:\n  \
                 {path}\nClaude Code runs the status line through Git Bash or PowerShell, and no \
                 quoting works in both. Install to a path without spaces and re-run."
            ),
        }
    }
}

/// The exact string to put in `statusLine.command`.
pub fn install_command(exe: &Path) -> Result<String, CommandError> {
    let path = exe.to_string_lossy().replace('\\', "/");
    let path = if path.contains(' ') {
        match short_path(exe) {
            Some(short) if !short.contains(' ') => short.replace('\\', "/"),
            _ => return Err(CommandError::UnquotablePath(path)),
        }
    } else {
        path
    };
    Ok(format!("{path} statusline"))
}

/// The DOS 8.3 name for a path, which by construction has no spaces.
///
/// Returns None when 8.3 generation is disabled on the volume, or the path does
/// not exist yet -- both of which the caller has to treat as "cannot install
/// here" rather than papering over.
#[cfg(windows)]
fn short_path(path: &Path) -> Option<String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut buffer = vec![0u16; 512];
    // SAFETY: `wide` is NUL-terminated and `buffer` is described by its length.
    let written =
        unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(buffer.as_mut_slice())) } as usize;
    if written == 0 || written >= buffer.len() {
        return None;
    }
    buffer.truncate(written);
    Some(
        std::ffi::OsString::from_wide(&buffer)
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(not(windows))]
fn short_path(_path: &Path) -> Option<String> {
    None
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub previous: Option<String>,
    pub command: String,
    /// The command we are preserving into `chain`, if any.
    pub chained: Option<String>,
    pub already_installed: bool,
}

/// What `apply` would do, without doing it.
///
/// When we're already installed the chained command has to be recovered from
/// our own settings and carried forward -- otherwise a second install replaces
/// the wrapper with a bare invocation and silently discards the user's original
/// status line. Rebuilding this way also repairs the path if the app moved.
pub fn plan(current: Option<&str>, existing_chain: Option<&str>, command: &str) -> Plan {
    let already_ours = current.map(is_ours).unwrap_or(false);
    let chained = if already_ours {
        existing_chain.map(str::to_string)
    } else {
        current.map(str::to_string).filter(|c| !c.trim().is_empty())
    };

    Plan {
        previous: current.map(str::to_string),
        command: command.to_string(),
        chained,
        already_installed: already_ours && current == Some(command),
    }
}

/// `settings_path` is Claude Code's settings file; `ours_path` is our own,
/// where the chained command is kept. Both are parameters rather than looked up
/// so the tests never have to touch a real profile or a shared env var.
pub fn apply(exe: &Path, settings_path: &Path, ours_path: &Path) -> Result<Plan, String> {
    let command = install_command(exe).map_err(|error| error.to_string())?;
    let mut claude = load(settings_path)?;
    let current = current_command(&claude);
    let mut ours = Settings::load_from(ours_path);

    let plan = plan(current.as_deref(), ours.chain.as_deref(), &command);

    ours.chain = plan.chained.clone();
    ours.save_to(ours_path)
        .map_err(|error| format!("could not save our settings: {error}"))?;

    claude["statusLine"] = json!({"type": "command", "command": command});
    save(settings_path, &claude)?;
    Ok(plan)
}

pub fn remove(settings_path: &Path, ours_path: &Path) -> Result<Option<String>, String> {
    let mut claude = load(settings_path)?;
    let current = current_command(&claude);

    match current.as_deref() {
        Some(command) if !is_ours(command) => {
            return Err("Status line is not managed by Claude Quota; nothing to remove.".into())
        }
        _ => {}
    }

    let mut ours = Settings::load_from(ours_path);
    let restored = ours.chain.take();
    ours.save_to(ours_path)
        .map_err(|error| format!("could not save our settings: {error}"))?;

    match &restored {
        Some(previous) => {
            claude["statusLine"] = json!({"type": "command", "command": previous});
        }
        None => {
            if let Some(object) = claude.as_object_mut() {
                object.remove("statusLine");
            }
        }
    }
    save(settings_path, &claude)?;
    Ok(restored)
}

fn current_command(claude: &Value) -> Option<String> {
    claude
        .get("statusLine")?
        .get("command")?
        .as_str()
        .map(str::to_string)
}

fn load(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let data = std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if data.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&data)
        .map_err(|error| format!("could not parse {} ({error})", path.display()))
}

/// Back up before writing. This file is the user's Claude Code configuration,
/// not ours, and a bad write costs them every setting in it.
fn save(path: &Path, settings: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if path.exists() {
        let backup = path.with_extension("json.quota-backup");
        std::fs::copy(path, &backup)
            .map_err(|error| format!("could not back up {}: {error}", path.display()))?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(path, json + "\n")
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_uses_forward_slashes_and_no_quoting() {
        let command = install_command(Path::new(
            r"C:\Users\tn\AppData\Local\ClaudeQuota\claude-quota.exe",
        ))
        .unwrap();
        assert_eq!(
            command,
            "C:/Users/tn/AppData/Local/ClaudeQuota/claude-quota.exe statusline"
        );

        // Every one of these breaks under one shell or the other.
        assert!(
            !command.contains('\\'),
            "Git Bash eats backslashes: {command}"
        );
        assert!(
            !command.contains('"'),
            "PowerShell will not execute a quoted path: {command}"
        );
        assert!(!command.contains('\''), "{command}");
        assert!(
            !command.contains('&'),
            "Git Bash cannot run PowerShell's call operator"
        );
        assert!(
            !command.contains('='),
            "a VAR=value prefix is bash-only syntax"
        );
        assert!(!command.contains(' ') || command.matches(' ').count() == 1);
    }

    #[test]
    fn a_path_with_a_space_is_refused_rather_than_written_broken() {
        // On a machine where 8.3 names are disabled there is nothing we can
        // write that works, and a silently broken status line is worse than a
        // message saying so.
        let result = install_command(Path::new(
            r"C:\Program Files\No Short Name\claude-quota.exe",
        ));
        if let Ok(command) = &result {
            // 8.3 was available; then it must have produced a space-free path.
            assert!(
                !command.trim_end_matches(" statusline").contains(' '),
                "{command}"
            );
        } else {
            assert!(matches!(result, Err(CommandError::UnquotablePath(_))));
        }
    }

    #[test]
    fn our_own_command_is_recognisable_wherever_it_was_installed() {
        assert!(is_ours(
            "C:/Users/tn/AppData/Local/ClaudeQuota/claude-quota.exe statusline"
        ));
        assert!(is_ours(
            "C:/USERS/TN/CLAUDEQUOTA/CLAUDE-QUOTA.EXE STATUSLINE"
        ));
        // The macOS bridge, in case someone shares a settings file.
        assert!(is_ours(
            "python ~/src/claude-quota/statusline/claude-quota-statusline.py"
        ));

        assert!(!is_ours("powershell -File C:/me/status.ps1"));
        assert!(
            !is_ours("claude-quota.exe"),
            "the app itself is not the bridge"
        );
    }

    #[test]
    fn an_existing_status_line_is_preserved_into_the_chain() {
        let plan = plan(
            Some("powershell -File C:/me/status.ps1"),
            None,
            "C:/x/claude-quota.exe statusline",
        );
        assert_eq!(
            plan.chained.as_deref(),
            Some("powershell -File C:/me/status.ps1")
        );
        assert!(!plan.already_installed);
    }

    #[test]
    fn installing_twice_does_not_discard_the_users_original_status_line() {
        // The bug this guards: rebuilding from the current command alone would
        // read our own wrapper as "the previous status line" and chain to it.
        let ours = "C:/x/claude-quota.exe statusline";
        let first = plan(Some("powershell -File C:/me/status.ps1"), None, ours);
        let second = plan(Some(ours), first.chained.as_deref(), ours);
        assert_eq!(second.chained, first.chained);
        assert!(second.already_installed);
    }

    #[test]
    fn reinstalling_at_a_new_path_repairs_the_command_and_keeps_the_chain() {
        let installed = plan(
            Some("C:/old/claude-quota.exe statusline"),
            Some("my-status"),
            "C:/new/claude-quota.exe statusline",
        );
        assert_eq!(installed.command, "C:/new/claude-quota.exe statusline");
        assert_eq!(installed.chained.as_deref(), Some("my-status"));
        assert!(!installed.already_installed);
    }

    #[test]
    fn a_blank_existing_command_is_not_worth_chaining_to() {
        let ours = "C:/x/claude-quota.exe statusline";
        assert_eq!(plan(Some("   "), None, ours).chained, None);
        assert_eq!(plan(None, None, ours).chained, None);
    }

    fn with_claude_settings(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        std::fs::write(&path, contents).unwrap();
        (directory, path)
    }

    #[test]
    fn applying_leaves_every_other_setting_untouched_and_backs_the_file_up() {
        let ours = tempfile::tempdir().unwrap();
        let ours = ours.path().join("settings.json");

        let (_directory, path) = with_claude_settings(
            r#"{"model": "opus", "statusLine": {"type": "command", "command": "my-status"}}"#,
        );
        let plan = apply(Path::new("C:/x/claude-quota.exe"), &path, &ours).unwrap();
        assert_eq!(plan.chained.as_deref(), Some("my-status"));

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["model"], "opus");
        assert_eq!(
            written["statusLine"]["command"],
            "C:/x/claude-quota.exe statusline"
        );
        assert!(path.with_extension("json.quota-backup").exists());

        // ...and removing puts the user's own command back.
        let restored = remove(&path, &ours).unwrap();
        assert_eq!(restored.as_deref(), Some("my-status"));
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["statusLine"]["command"], "my-status");
        assert_eq!(written["model"], "opus");
    }

    #[test]
    fn removing_when_there_was_nothing_before_drops_the_setting_entirely() {
        let ours = tempfile::tempdir().unwrap();
        let ours = ours.path().join("settings.json");

        let (_directory, path) = with_claude_settings(r#"{"model": "opus"}"#);
        apply(Path::new("C:/x/claude-quota.exe"), &path, &ours).unwrap();
        assert_eq!(remove(&path, &ours).unwrap(), None);

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(written.get("statusLine").is_none());
        assert_eq!(written["model"], "opus");
    }

    #[test]
    fn we_refuse_to_remove_a_status_line_that_is_not_ours() {
        let (_directory, path) = with_claude_settings(
            r#"{"statusLine": {"type": "command", "command": "powershell -File x.ps1"}}"#,
        );
        let ours = tempfile::tempdir().unwrap();
        assert!(remove(&path, &ours.path().join("settings.json")).is_err());
    }

    #[test]
    fn a_missing_or_empty_settings_file_is_created_rather_than_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let ours = directory.path().join("ours.json");
        apply(Path::new("C:/x/claude-quota.exe"), &path, &ours).unwrap();

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written["statusLine"]["command"],
            "C:/x/claude-quota.exe statusline"
        );
    }

    #[test]
    fn a_settings_file_we_cannot_parse_is_reported_rather_than_overwritten() {
        let (_directory, path) = with_claude_settings("{ not json");
        let ours = tempfile::tempdir().unwrap();
        assert!(apply(
            Path::new("C:/x/claude-quota.exe"),
            &path,
            &ours.path().join("o.json")
        )
        .is_err());
        // Still exactly as the user left it.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }
}
