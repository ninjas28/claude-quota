//! The status line bridge, as a subcommand: `claude-quota.exe statusline`.
//!
//! Port of `statusline/claude-quota-statusline.py`. Claude Code hands its status
//! line command a JSON blob on stdin that includes a `rate_limits` object, with
//! no network call of its own. This caches that object to disk so the tray app
//! can read your live quota for free, then prints a status line so you lose
//! nothing by installing it.
//!
//! Nothing here consumes quota, makes a network call, or touches credentials.
//!
//! Why a subcommand rather than shipping the Python script: Claude Code on
//! Windows runs the status line through Git Bash when it is installed and
//! PowerShell when it isn't, and `python` is not reliably on PATH in both. A
//! single self-contained exe removes the interpreter from the equation. The
//! Python bridge still works and writes the identical file -- see the README.
//!
//! Every failure mode is swallowed. A status line that raises would show an
//! error in Claude Code on every render, and caching quota is never worth that.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

use crate::settings::Settings;
use crate::status_line_cache::cache_path;

pub fn main() {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);

    let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);

    if data.is_object() {
        // Caching is best effort; never break the status line over it.
        let _ = write_cache(&data, &cache_path());
    }

    if let Some(chain) = chained_command() {
        if let Some(output) = run_chained(&chain, &raw) {
            println!("{output}");
            return;
        }
        // Fall through to our own rendering.
    }

    println!("{}", default_statusline(&data));
}

/// The status line that was configured before we took over.
///
/// `CLAUDE_QUOTA_CHAIN` keeps parity with the macOS installer for anyone who
/// sets it by hand; the settings file is what our own installer writes, because
/// the env-var prefix form is bash-only syntax.
fn chained_command() -> Option<String> {
    if let Ok(chain) = std::env::var("CLAUDE_QUOTA_CHAIN") {
        if !chain.trim().is_empty() {
            return Some(chain);
        }
    }
    Settings::load().chain.filter(|chain| !chain.trim().is_empty())
}

/// Cache the rate limit windows, atomically.
///
/// The tray app watches this path, and a rename is the only way to swap the
/// contents without it ever observing a half-written file.
pub fn write_cache(data: &Value, target: &Path) -> std::io::Result<()> {
    let rate_limits = data.get("rate_limits");

    let mut payload = serde_json::Map::new();
    for key in ["five_hour", "seven_day"] {
        let Some(window) = rate_limits.and_then(|limits| limits.get(key)) else { continue };
        if !window.is_object() {
            continue;
        }
        // A window without a usable number is not a window. Note `as_f64`
        // declines a boolean, which is what keeps a 0% window from being
        // confused with `false`.
        let Some(used) = window.get("used_percentage").and_then(Value::as_f64) else { continue };
        payload.insert(
            key.to_string(),
            json!({
                "used_percentage": used,
                "resets_at": window.get("resets_at").cloned().unwrap_or(Value::Null),
            }),
        );
    }

    // Nothing useful this render -- keep whatever we cached last time.
    if payload.is_empty() {
        return Ok(());
    }

    payload.insert("captured_at".into(), json!(unix_now()));
    payload.insert(
        "model".into(),
        data.get("model").and_then(|m| m.get("display_name")).cloned().unwrap_or(Value::Null),
    );
    payload.insert("session_id".into(), data.get("session_id").cloned().unwrap_or(Value::Null));

    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;

    // Same directory, so the rename stays on one volume and stays atomic.
    // The pid keeps two concurrent sessions from writing the same temp file.
    let temp = directory.join(format!(".quota-bar-{}.tmp", std::process::id()));
    let write = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(serde_json::to_string(&Value::Object(payload))?.as_bytes())?;
        file.sync_all()
    })();

    if let Err(error) = write {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }

    if let Err(error) = std::fs::rename(&temp, target) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn bar(percent: f64, width: usize) -> String {
    let filled = (percent.clamp(0.0, 100.0) / 100.0 * width as f64) as usize;
    "\u{2593}".repeat(filled) + &"\u{2591}".repeat(width - filled)
}

/// A compact default: model, context, and both quota windows.
pub fn default_statusline(data: &Value) -> String {
    let model = data
        .get("model")
        .and_then(|model| model.get("display_name"))
        .and_then(Value::as_str)
        .unwrap_or("Claude");
    let mut parts = vec![model.to_string()];

    if let Some(context) =
        data.get("context_window").and_then(|w| w.get("used_percentage")).and_then(Value::as_f64)
    {
        parts.push(format!("{} {}% ctx", bar(context, 10), context as i64));
    }

    let rate_limits = data.get("rate_limits");
    let quota: Vec<String> = [("five_hour", "5h"), ("seven_day", "7d")]
        .into_iter()
        .filter_map(|(key, label)| {
            let used = rate_limits?.get(key)?.get("used_percentage")?.as_f64()?;
            Some(format!("{} {}%", label, used as i64))
        })
        .collect();
    if !quota.is_empty() {
        parts.push(quota.join(" "));
    }

    parts.join(" | ")
}

/// Hand the original stdin to the user's existing status line command.
///
/// Returns None if the command fails, times out, or prints nothing, so the
/// caller falls back to our own rendering: a shell does not fail loudly on a
/// missing command -- it exits non-zero with empty output -- and returning that
/// verbatim would leave a permanently blank status line with no hint as to why.
///
/// The chained command runs through `cmd.exe` regardless of which shell invoked
/// us, so its meaning does not change depending on whether Git Bash happens to
/// be installed.
pub fn run_chained(command: &str, raw: &str) -> Option<String> {
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
    let mut child = Command::new(comspec)
        .arg("/C")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Both pipes get their own thread: writing stdin and then reading stdout on
    // this thread deadlocks the moment the child's output fills the pipe buffer
    // before it has finished reading its input.
    if let Some(mut stdin) = child.stdin.take() {
        let payload = raw.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(payload.as_bytes());
        });
    }

    let (sender, receiver) = std::sync::mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut output = String::new();
            let _ = stdout.read_to_string(&mut output);
            let _ = sender.send(output);
        });
    }

    let output = match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(output) => output,
        Err(_) => {
            let _ = child.kill();
            return None;
        }
    };

    let status = child.wait().ok()?;
    let output = output.trim_end_matches(['\n', '\r']).to_string();
    if !status.success() || output.trim().is_empty() {
        return None;
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(five: f64, seven: f64) -> Value {
        json!({
            "model": {"display_name": "Opus 5"},
            "session_id": "sess-abc123",
            "context_window": {"used_percentage": 34, "context_window_size": 200000},
            "rate_limits": {
                "five_hour": {"used_percentage": five, "resets_at": 1738425600},
                "seven_day": {"used_percentage": seven, "resets_at": 1738857600}
            }
        })
    }

    #[test]
    fn the_cache_carries_every_field_the_reader_looks_for() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("quota-bar-cache.json");
        write_cache(&payload(23.5, 41.2), &target).unwrap();

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(written["five_hour"]["used_percentage"], 23.5);
        assert_eq!(written["five_hour"]["resets_at"], 1_738_425_600i64);
        assert_eq!(written["seven_day"]["used_percentage"], 41.2);
        assert_eq!(written["model"], "Opus 5");
        assert_eq!(written["session_id"], "sess-abc123");
        assert!(written["captured_at"].as_f64().unwrap() > 1_700_000_000.0);

        // The end-to-end contract: what we write, the app reads.
        let (snapshot, context) = crate::status_line_cache::read_at(&target).unwrap();
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(context.model.as_deref(), Some("Opus 5"));
    }

    #[test]
    fn a_render_with_no_rate_limits_leaves_the_previous_cache_alone() {
        // `rate_limits` only appears after the first API response of a session.
        // Blanking the cache on every render before that would make the tray
        // icon flicker to "unknown" at the start of every session.
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("quota-bar-cache.json");

        write_cache(&payload(23.5, 41.2), &target).unwrap();
        write_cache(&json!({"model": {"display_name": "Opus 5"}}), &target).unwrap();

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(written["five_hour"]["used_percentage"], 23.5);
    }

    #[test]
    fn a_window_at_zero_percent_is_cached_rather_than_read_as_absent() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("quota-bar-cache.json");
        write_cache(&payload(0.0, 0.0), &target).unwrap();

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(written["five_hour"]["used_percentage"], 0.0);
    }

    #[test]
    fn a_window_whose_percentage_is_the_wrong_type_is_skipped_not_guessed_at() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("quota-bar-cache.json");

        write_cache(
            &json!({"rate_limits": {
                "five_hour": {"used_percentage": true},
                "seven_day": {"used_percentage": "41.2"}
            }}),
            &target,
        )
        .unwrap();
        assert!(!target.exists(), "nothing usable, so nothing written");
    }

    #[test]
    fn a_rate_limits_field_of_the_wrong_type_does_not_panic() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("quota-bar-cache.json");
        for shape in [json!("nope"), json!([1, 2]), json!(null), json!(7)] {
            write_cache(&json!({ "rate_limits": shape }), &target).unwrap();
        }
        assert!(!target.exists());
    }

    #[test]
    fn no_temp_files_are_left_behind() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("quota-bar-cache.json");
        write_cache(&payload(23.5, 41.2), &target).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn the_default_status_line_shows_model_context_and_both_windows() {
        let rendered = default_statusline(&payload(23.5, 41.2));
        assert!(rendered.starts_with("Opus 5 | "), "{rendered}");
        assert!(rendered.contains("34% ctx"), "{rendered}");
        assert!(rendered.ends_with("5h 23% 7d 41%"), "{rendered}");
    }

    #[test]
    fn the_default_status_line_degrades_field_by_field() {
        assert_eq!(default_statusline(&json!({})), "Claude");
        assert_eq!(default_statusline(&Value::Null), "Claude");
        assert_eq!(
            default_statusline(&json!({"model": {"display_name": "Opus 5"}})),
            "Opus 5"
        );
        assert_eq!(
            default_statusline(&json!({
                "model": {"display_name": "Opus 5"},
                "rate_limits": {"five_hour": {"used_percentage": 23.5}}
            })),
            "Opus 5 | 5h 23%"
        );
    }

    #[test]
    fn the_context_bar_fills_proportionally_and_never_overflows() {
        assert_eq!(bar(0.0, 10).chars().filter(|c| *c == '\u{2593}').count(), 0);
        assert_eq!(bar(50.0, 10).chars().filter(|c| *c == '\u{2593}').count(), 5);
        assert_eq!(bar(100.0, 10).chars().filter(|c| *c == '\u{2593}').count(), 10);
        assert_eq!(bar(140.0, 10).chars().count(), 10);
        assert_eq!(bar(-5.0, 10).chars().count(), 10);
    }

    #[cfg(windows)]
    #[test]
    fn a_chained_command_receives_the_untouched_stdin_and_its_output_is_used() {
        let raw = r#"{"model": {"display_name": "Opus 5"}}"#;
        // `more` echoes stdin verbatim on Windows.
        let output = run_chained("more", raw).unwrap();
        assert!(output.contains("Opus 5"), "{output}");
    }

    #[cfg(windows)]
    #[test]
    fn a_chained_command_that_fails_or_says_nothing_falls_back_to_our_rendering() {
        assert_eq!(run_chained("exit 1", "{}"), None);
        assert_eq!(run_chained("no-such-command-anywhere", "{}"), None);
        assert_eq!(run_chained("echo.", "{}"), None, "whitespace only is not a status line");
    }
}
