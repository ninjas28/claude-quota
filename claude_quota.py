#!/usr/bin/env python3
"""
Claude Max Quota Checker

Checks your Claude Max subscription utilization by running a minimal
Claude Code CLI call through a local proxy to capture the rate-limit
response headers that Anthropic returns.

Usage:
    python3 claude_quota.py          # Pretty-printed summary
    python3 claude_quota.py --json   # Machine-readable JSON output

Requires:
    - Claude Code CLI (`claude`) installed and logged in
    - macOS (uses security keychain for Claude Code auth)
"""

import http.server
import json
import os
import socket
import subprocess
import sys
import threading
from datetime import datetime
from http.client import HTTPSConnection


ANTHROPIC_API_HOST = "api.anthropic.com"
RATE_LIMIT_PREFIX = "anthropic-ratelimit-unified-"


def find_free_port():
    """Find a free port on localhost."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class HeaderCapturingHandler(http.server.BaseHTTPRequestHandler):
    """HTTP proxy handler that forwards to Anthropic and captures rate-limit headers."""

    captured_headers = {}

    def log_message(self, format_string, *args):
        pass  # Silence request logging

    def handle(self):
        try:
            super().handle()
        except ConnectionResetError:
            pass  # Expected when --bare fallback retries

    def do_POST(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length)

        # Forward to Anthropic over HTTPS
        connection = HTTPSConnection(ANTHROPIC_API_HOST, 443)
        forward_headers = {}
        for key, value in self.headers.items():
            lower_key = key.lower()
            if lower_key not in ("host", "connection", "transfer-encoding"):
                forward_headers[key] = value
        forward_headers["Host"] = ANTHROPIC_API_HOST

        connection.request("POST", self.path, body=body, headers=forward_headers)
        response = connection.getresponse()

        # Capture rate-limit headers
        for key, value in response.getheaders():
            if RATE_LIMIT_PREFIX in key.lower():
                clean_key = key.lower().replace(RATE_LIMIT_PREFIX, "")
                try:
                    HeaderCapturingHandler.captured_headers[clean_key] = float(value)
                except ValueError:
                    HeaderCapturingHandler.captured_headers[clean_key] = value

        # Forward response back to Claude Code
        self.send_response(response.status)
        for key, value in response.getheaders():
            self.send_header(key, value)
        self.end_headers()

        # Stream the response body through
        while True:
            chunk = response.read(8192)
            if not chunk:
                break
            self.wfile.write(chunk)

        connection.close()


def check_quota():
    """Start a proxy, run a minimal claude call through it, return rate-limit data."""
    port = find_free_port()
    server = http.server.HTTPServer(("127.0.0.1", port), HeaderCapturingHandler)
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    try:
        result = subprocess.run(
            [
                "claude",
                "-p",
                "--output-format", "json",
                "--tools", "",
                "--model", "haiku",
                "--no-session-persistence",
                "--bare",
                "hi",
            ],
            capture_output=True,
            text=True,
            timeout=30,
            env={**os.environ, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{port}"},
        )

        if result.returncode != 0:
            # --bare skips OAuth — fall back without it
            result = subprocess.run(
                [
                    "claude",
                    "-p",
                    "--output-format", "json",
                    "--tools", "",
                    "--model", "haiku",
                    "--no-session-persistence",
                    "hi",
                ],
                capture_output=True,
                text=True,
                timeout=60,
                env={**os.environ, "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{port}"},
            )

        if not HeaderCapturingHandler.captured_headers:
            stderr_output = result.stderr.strip()
            stdout_output = result.stdout.strip()
            error_detail = stderr_output or stdout_output or "no output"
            raise RuntimeError(
                f"No rate-limit headers captured. Claude exit code: {result.returncode}. "
                f"Detail: {error_detail[:200]}"
            )

        return HeaderCapturingHandler.captured_headers

    finally:
        server.shutdown()


def format_timestamp(epoch_seconds):
    """Convert epoch seconds to a human-readable local time string."""
    dt = datetime.fromtimestamp(epoch_seconds)
    now = datetime.now()
    delta = dt - now

    if delta.total_seconds() < 0:
        return f"{dt.strftime('%a %b %d %I:%M %p')} (already reset)"
    elif delta.total_seconds() < 3600:
        return f"{dt.strftime('%a %b %d %I:%M %p')} (in {int(delta.total_seconds() / 60)} min)"
    elif delta.total_seconds() < 86400:
        hours = delta.total_seconds() / 3600
        return f"{dt.strftime('%a %b %d %I:%M %p')} (in {hours:.1f} hrs)"
    else:
        days = delta.total_seconds() / 86400
        return f"{dt.strftime('%a %b %d %I:%M %p')} (in {days:.1f} days)"


def print_summary(data):
    """Print a human-readable quota summary."""
    five_hour_utilization = data.get("5h-utilization", 0)
    seven_day_utilization = data.get("7d-utilization", 0)
    overage_utilization = data.get("overage-utilization", 0)

    five_hour_reset = data.get("5h-reset", 0)
    seven_day_reset = data.get("7d-reset", 0)

    five_hour_status = data.get("5h-status", "unknown")
    seven_day_status = data.get("7d-status", "unknown")
    overage_status = data.get("overage-status", "unknown")
    fallback = data.get("fallback", "unknown")
    fallback_percentage = data.get("fallback-percentage", 0)

    five_hour_percent = five_hour_utilization * 100
    seven_day_percent = seven_day_utilization * 100
    overage_percent = overage_utilization * 100
    five_hour_remaining = max(0, (1 - five_hour_utilization)) * 100
    seven_day_remaining = max(0, (1 - seven_day_utilization)) * 100

    def bar(percent, width=30):
        filled = int(percent / 100 * width)
        return f"[{'█' * filled}{'·' * (width - filled)}]"

    print()
    print("  Claude Max Quota")
    print("  " + "=" * 48)
    print()
    print(f"  5-Hour Window:  {bar(five_hour_percent)} {five_hour_percent:.1f}%")
    print(f"                  Remaining: {five_hour_remaining:.1f}%")
    if five_hour_reset:
        print(f"                  Resets: {format_timestamp(five_hour_reset)}")
    print(f"                  Status: {five_hour_status}")
    print()
    print(f"  7-Day Window:   {bar(seven_day_percent)} {seven_day_percent:.1f}%")
    print(f"                  Remaining: {seven_day_remaining:.1f}%")
    if seven_day_reset:
        print(f"                  Resets: {format_timestamp(seven_day_reset)}")
    print(f"                  Status: {seven_day_status}")
    print()
    print(f"  Overage:        {bar(overage_percent)} {overage_percent:.1f}%")
    print(f"                  Status: {overage_status}")
    if isinstance(fallback_percentage, (int, float)):
        print(f"                  Fallback: {fallback} ({fallback_percentage * 100:.0f}%)")
    else:
        print(f"                  Fallback: {fallback}")
    print()


def main():
    json_output = "--json" in sys.argv

    try:
        rate_limit_data = check_quota()

        if json_output:
            print(json.dumps(rate_limit_data, indent=2))
        else:
            print_summary(rate_limit_data)

    except KeyboardInterrupt:
        sys.exit(130)
    except Exception as error:
        if json_output:
            print(json.dumps({"error": str(error)}), file=sys.stderr)
        else:
            print(f"Error: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
