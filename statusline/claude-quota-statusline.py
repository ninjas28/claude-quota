#!/usr/bin/env python3
"""
Claude Quota Bar -- status line bridge.

Claude Code hands its status line command a JSON blob on stdin that includes a
`rate_limits` object, with no network call of its own. This script caches that
object to disk so the menu bar app can read your live quota for free, then
prints a status line so you lose nothing by installing it.

Install by pointing `statusLine.command` at this file in ~/.claude/settings.json:

    {
      "statusLine": {
        "type": "command",
        "command": "/path/to/claude-quota-statusline.py"
      }
    }

Already have a status line? Keep it -- set CLAUDE_QUOTA_CHAIN to your existing
command and this script will feed it the untouched stdin and print its output
instead of its own:

    {
      "statusLine": {
        "type": "command",
        "command": "CLAUDE_QUOTA_CHAIN=~/.claude/my-statusline.sh /path/to/claude-quota-statusline.py"
      }
    }

Nothing here consumes quota, makes a network call, or touches credentials.

Notes:
  - `rate_limits` only appears for Claude.ai Pro/Max accounts, and only after
    the first API response in a session. Before then there is nothing to cache
    and the previous cache file is left untouched.
  - Every failure mode is swallowed. A status line that raises would show an
    error in Claude Code on every render, and caching quota is never worth that.
"""

import json
import os
import subprocess
import sys
import tempfile
import time

DEFAULT_CACHE = "~/.claude/quota-bar-cache.json"


def cache_path():
    return os.path.expanduser(os.environ.get("CLAUDE_QUOTA_CACHE") or DEFAULT_CACHE)


def write_cache(data):
    """Cache the rate limit windows, atomically.

    The menu bar app watches this path, and a rename is the only way to swap the
    contents without it ever observing a half-written file.
    """
    rate_limits = data.get("rate_limits") or {}

    windows = {}
    for key in ("five_hour", "seven_day"):
        window = rate_limits.get(key)
        if not isinstance(window, dict):
            continue
        used = window.get("used_percentage")
        if not isinstance(used, (int, float)):
            continue
        windows[key] = {
            "used_percentage": float(used),
            "resets_at": window.get("resets_at"),
        }

    # Nothing useful this render -- keep whatever we cached last time.
    if not windows:
        return

    payload = dict(windows)
    payload["captured_at"] = time.time()
    payload["model"] = (data.get("model") or {}).get("display_name")
    payload["session_id"] = data.get("session_id")

    target = cache_path()
    directory = os.path.dirname(target)
    os.makedirs(directory, exist_ok=True)

    handle, temp_path = tempfile.mkstemp(dir=directory, prefix=".quota-bar-", suffix=".tmp")
    try:
        with os.fdopen(handle, "w") as file:
            json.dump(payload, file)
        os.replace(temp_path, target)
    except Exception:
        try:
            os.unlink(temp_path)
        except OSError:
            pass
        raise


def bar(percent, width=10):
    filled = int(max(0.0, min(100.0, percent)) / 100 * width)
    return "▓" * filled + "░" * (width - filled)


def default_statusline(data):
    """A compact default: model, context, and both quota windows."""
    model = (data.get("model") or {}).get("display_name") or "Claude"
    parts = [model]

    context = (data.get("context_window") or {}).get("used_percentage")
    if isinstance(context, (int, float)):
        parts.append("%s %d%% ctx" % (bar(context), int(context)))

    rate_limits = data.get("rate_limits") or {}
    quota = []
    for key, label in (("five_hour", "5h"), ("seven_day", "7d")):
        window = rate_limits.get(key)
        if isinstance(window, dict) and isinstance(window.get("used_percentage"), (int, float)):
            quota.append("%s %d%%" % (label, int(window["used_percentage"])))
    if quota:
        parts.append(" ".join(quota))

    return " | ".join(parts)


def run_chained(command, raw):
    """Hand the original stdin to the user's existing status line command."""
    result = subprocess.run(
        os.path.expanduser(command),
        shell=True,
        input=raw,
        capture_output=True,
        text=True,
        timeout=5,
    )
    return result.stdout.rstrip("\n")


def main():
    raw = sys.stdin.read()

    try:
        data = json.loads(raw)
    except Exception:
        data = {}

    if data:
        try:
            write_cache(data)
        except Exception:
            pass  # Caching is best effort; never break the status line over it.

    chain = os.environ.get("CLAUDE_QUOTA_CHAIN")
    if chain:
        try:
            print(run_chained(chain, raw))
            return
        except Exception:
            pass  # Fall through to our own rendering.

    try:
        print(default_statusline(data))
    except Exception:
        print("")


if __name__ == "__main__":
    main()
