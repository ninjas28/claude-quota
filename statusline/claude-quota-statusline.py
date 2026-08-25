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


def as_dict(value):
    """Nested lookups, tolerant of a field arriving as the wrong type.

    `x.get("a") or {}` is not enough: a non-empty string is truthy and then
    `.get` raises, which would blank the whole status line.
    """
    return value if isinstance(value, dict) else {}


def write_cache(data):
    """Cache the rate limit windows, atomically.

    The menu bar app watches this path, and a rename is the only way to swap the
    contents without it ever observing a half-written file.
    """
    rate_limits = as_dict(data.get("rate_limits"))

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
    payload["model"] = as_dict(data.get("model")).get("display_name")
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
    model = as_dict(data.get("model")).get("display_name") or "Claude"
    parts = [model]

    context = as_dict(data.get("context_window")).get("used_percentage")
    if isinstance(context, (int, float)):
        parts.append("%s %d%% ctx" % (bar(context), int(context)))

    rate_limits = as_dict(data.get("rate_limits"))
    quota = []
    for key, label in (("five_hour", "5h"), ("seven_day", "7d")):
        window = rate_limits.get(key)
        if isinstance(window, dict) and isinstance(window.get("used_percentage"), (int, float)):
            quota.append("%s %d%%" % (label, int(window["used_percentage"])))
    if quota:
        parts.append(" ".join(quota))

    return " | ".join(parts)


def run_chained(command, raw):
    """Hand the original stdin to the user's existing status line command.

    Raises if the command fails or prints nothing, so the caller falls back to
    our own rendering. `shell=True` does not raise on a missing command -- it
    just exits 127 with empty output -- and returning that verbatim would leave
    a permanently blank status line with no hint as to why.
    """
    result = subprocess.run(
        os.path.expanduser(command),
        shell=True,
        input=raw,
        capture_output=True,
        text=True,
        timeout=5,
    )
    output = result.stdout.rstrip("\n")
    if result.returncode != 0 or not output.strip():
        raise RuntimeError("chained status line failed (exit %d)" % result.returncode)
    return output


def main():
    # The bar is drawn with block characters, and on Windows a piped stdout
    # defaults to the locale codepage -- cp1252, which cannot encode them. The
    # print then raises, the blanket except below swallows it, and the status
    # line comes out empty while the cache carries on working: the one failure
    # mode that looks like the script is fine. Ask for UTF-8, which is what
    # reads the output anyway.
    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass  # Python < 3.7, or a stdout that cannot be reconfigured.

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
