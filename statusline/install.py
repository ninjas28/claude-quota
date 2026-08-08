#!/usr/bin/env python3
"""
Wires the status line bridge into ~/.claude/settings.json.

    ./install.py            show what would change
    ./install.py --apply    make the change (backs up settings.json first)
    ./install.py --remove   restore whatever status line you had before

If you already have a status line configured, it is preserved: your existing
command moves into CLAUDE_QUOTA_CHAIN, so the bridge feeds it the untouched
stdin and prints its output. Your status line looks exactly the same afterwards.
"""

import json
import os
import shutil
import sys

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "claude-quota-statusline.py")
SETTINGS = os.path.expanduser("~/.claude/settings.json")
MARKER = "claude-quota-statusline.py"


def load_settings():
    if not os.path.exists(SETTINGS):
        return {}
    try:
        with open(SETTINGS) as file:
            return json.load(file)
    except Exception as error:
        sys.exit("error: could not parse %s (%s)" % (SETTINGS, error))


def build_command(current):
    """Our script, chaining to whatever status line was there before.

    When we're already installed, the chained command has to be recovered from
    the existing setting and carried forward -- otherwise a second --apply
    replaces the wrapper with a bare invocation and silently discards the
    user's original status line. Rebuilding this way also repairs the path if
    the repo moved.
    """
    previous = unwrap(current) if current and MARKER in current else current
    if previous and MARKER not in previous:
        return "CLAUDE_QUOTA_CHAIN=%s %s" % (json.dumps(previous), SCRIPT)
    return SCRIPT


def unwrap(command):
    """Recover the original command from a chained one, if present."""
    if not command or MARKER not in command:
        return None
    marker = "CLAUDE_QUOTA_CHAIN="
    if not command.startswith(marker):
        return None
    remainder = command[len(marker):]
    try:
        decoded, _ = json.JSONDecoder().raw_decode(remainder)
        return decoded
    except ValueError:
        return None


def save(settings):
    os.makedirs(os.path.dirname(SETTINGS), exist_ok=True)
    if os.path.exists(SETTINGS):
        backup = SETTINGS + ".quota-bar-backup"
        shutil.copy2(SETTINGS, backup)
        print("Backed up existing settings to %s" % backup)
    with open(SETTINGS, "w") as file:
        json.dump(settings, file, indent=2)
        file.write("\n")


def main():
    apply_changes = "--apply" in sys.argv
    remove = "--remove" in sys.argv

    if not os.path.exists(SCRIPT):
        sys.exit("error: %s not found" % SCRIPT)
    if not os.access(SCRIPT, os.X_OK):
        sys.exit("error: %s is not executable (run: chmod +x %s)" % (SCRIPT, SCRIPT))

    settings = load_settings()
    status_line = settings.get("statusLine") or {}
    current = status_line.get("command")

    if remove:
        restored = unwrap(current)
        if current and MARKER not in current:
            sys.exit("Status line is not managed by Claude Quota Bar; nothing to remove.")
        if restored:
            settings["statusLine"] = {"type": "command", "command": restored}
            print("Restore status line to: %s" % restored)
        else:
            settings.pop("statusLine", None)
            print("Remove the statusLine setting entirely")
        if apply_changes:
            save(settings)
            print("Done.")
        else:
            print("\nRe-run with --apply --remove to make this change.")
        return

    command = build_command(current)

    if current == command:
        print("Already installed. Nothing to do.")
        return

    print("Settings file: %s" % SETTINGS)
    if current:
        print("Current status line: %s" % current)
    print("New status line:     %s" % command)
    if current and MARKER not in current:
        print("\nYour existing status line is preserved and will still render.")

    settings["statusLine"] = {"type": "command", "command": command}

    if apply_changes:
        save(settings)
        print("\nDone. Start or resume a Claude Code session to populate the cache.")
    else:
        print("\nRe-run with --apply to make this change.")


if __name__ == "__main__":
    main()
