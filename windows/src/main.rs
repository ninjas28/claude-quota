//! Claude usage in the Windows notification area.
//!
//! With no arguments this is the tray app. `statusline` is the bridge Claude
//! Code calls on every render; the rest is what `install.ps1` drives.

// The tray app must never flash a console window. Inherited handles still work,
// so the bridge prints into Claude Code's pipe as normal, and `attach_parent_console`
// covers a human running a subcommand from a prompt.
#![windows_subsystem = "windows"]

use std::path::PathBuf;

use claude_quota::{app, bridge, paths, platform, statusline_install};

const USAGE: &str = "\
claude-quota -- Claude usage in the notification area

  claude-quota                     run the tray app
  claude-quota statusline          status line bridge (Claude Code calls this)
  claude-quota install-statusline  point ~/.claude/settings.json at the bridge
  claude-quota remove-statusline   restore whatever status line was there before
  claude-quota where               print the paths this build uses
";

fn main() {
    let command = std::env::args().nth(1);

    // The bridge is the hot path -- Claude Code runs it on every render -- and
    // it must not touch the console: its stdout is the status line.
    if command.as_deref() == Some("statusline") {
        bridge::main();
        return;
    }

    let Some(command) = command else {
        if let Err(error) = app::run() {
            eprintln!("claude-quota: {error}");
            std::process::exit(1);
        }
        return;
    };

    platform::attach_parent_console();

    let code = match command.as_str() {
        "install-statusline" => install(),
        "remove-statusline" => remove(),
        "where" => {
            println!("executable   {}", exe().display());
            println!("claude home  {}", paths::claude_home().display());
            println!("settings     {}", paths::settings_file().display());
            println!("cache        {}", claude_quota::status_line_cache::cache_path().display());
            0
        }
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
            0
        }
        other => {
            eprintln!("claude-quota: unknown command {other:?}\n\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("claude-quota.exe"))
}

fn install() -> i32 {
    match statusline_install::apply(&exe(), &paths::claude_settings(), &paths::settings_file()) {
        Err(error) => {
            eprintln!("claude-quota: {error}");
            1
        }
        Ok(plan) => {
            if plan.already_installed {
                println!("Already installed. Nothing to do.");
                return 0;
            }
            println!("Settings file: {}", paths::claude_settings().display());
            if let Some(previous) = &plan.previous {
                println!("Current status line: {previous}");
            }
            println!("New status line:     {}", plan.command);
            if let Some(chained) = &plan.chained {
                println!("\nYour existing status line is preserved and will still render.");
                println!("It runs through cmd.exe as: {chained}");
            }
            println!("\nDone. Start or resume a Claude Code session to populate the cache.");
            0
        }
    }
}

fn remove() -> i32 {
    match statusline_install::remove(&paths::claude_settings(), &paths::settings_file()) {
        Err(error) => {
            eprintln!("claude-quota: {error}");
            1
        }
        Ok(Some(restored)) => {
            println!("Restored status line: {restored}");
            0
        }
        Ok(None) => {
            println!("Removed the statusLine setting entirely.");
            0
        }
    }
}
