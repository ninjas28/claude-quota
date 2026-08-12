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

Note: these headers report utilization as a 0-1 fraction. The OAuth usage
endpoint the menu bar app uses reports the same numbers as 0-100 percentages.
Same data, different scale -- don't mix them up.
"""

import http.server
import json
import os
import subprocess
import sys
import threading
from datetime import datetime
from http.client import HTTPSConnection


ANTHROPIC_API_HOST = "api.anthropic.com"
RATE_LIMIT_PREFIX = "anthropic-ratelimit-unified-"
UPSTREAM_TIMEOUT = 30

# Headers that describe a single connection and must not be relayed.
# Transfer-Encoding matters most here: http.client strips chunk framing before
# we ever see the body, so forwarding the header would tell our client to
# de-chunk a stream that has already been de-chunked.
HOP_BY_HOP_HEADERS = frozenset({
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
})


class HeaderCapturingHandler(http.server.BaseHTTPRequestHandler):
    """HTTP proxy handler that forwards to Anthropic and captures rate-limit headers.

    Captured headers live on the *server* instance, not on the class. A class
    attribute survives for the life of the process, so a second check would see
    the first one's numbers and a failed check would quietly report stale data
    instead of raising.
    """

    def log_message(self, format_string, *args):
        pass  # Silence request logging

    def handle(self):
        try:
            super().handle()
        except ConnectionResetError:
            pass  # Expected when the client gives up mid-request

    def do_POST(self):
        self.forward()

    def do_GET(self):
        self.forward()

    def forward(self):
        """Relay one request upstream, recording any rate-limit headers."""
        responded = False
        try:
            content_length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(content_length) if content_length else None

            forward_headers = {
                key: value
                for key, value in self.headers.items()
                if key.lower() not in HOP_BY_HOP_HEADERS and key.lower() != "host"
            }
            forward_headers["Host"] = ANTHROPIC_API_HOST

            connection = HTTPSConnection(
                ANTHROPIC_API_HOST, 443, timeout=UPSTREAM_TIMEOUT
            )
            try:
                connection.request(self.command, self.path, body=body, headers=forward_headers)
                response = connection.getresponse()

                self.capture(response.getheaders())

                self.send_response(response.status)
                for key, value in response.getheaders():
                    if key.lower() not in HOP_BY_HOP_HEADERS:
                        self.send_header(key, value)
                self.end_headers()
                responded = True

                while True:
                    chunk = response.read(8192)
                    if not chunk:
                        break
                    self.wfile.write(chunk)
            finally:
                connection.close()

        except Exception as error:
            # Without this the handler thread dies and Claude Code sees a
            # dropped socket, which surfaces as a confusing generic failure.
            if not responded:
                try:
                    self.send_error(502, "proxy error: %s" % error)
                except Exception:
                    pass

    def capture(self, headers):
        captured = self.server.captured_headers
        for key, value in headers:
            lower_key = key.lower()
            if not lower_key.startswith(RATE_LIMIT_PREFIX):
                continue
            clean_key = lower_key[len(RATE_LIMIT_PREFIX):]
            try:
                captured[clean_key] = float(value)
            except ValueError:
                captured[clean_key] = value


class CapturingProxy(http.server.HTTPServer):
    """An HTTPServer that owns the headers its handlers capture."""

    def __init__(self):
        # Bind port 0 and read back what the OS gave us. Picking a free port in
        # a separate socket and reopening it leaves a window for something else
        # to take it first.
        super().__init__(("127.0.0.1", 0), HeaderCapturingHandler)
        self.captured_headers = {}

    @property
    def base_url(self):
        return "http://127.0.0.1:%d" % self.server_address[1]


def claude_command(bare):
    command = [
        "claude",
        "-p",
        "--output-format", "json",
        "--tools", "",
        "--model", "haiku",
        "--no-session-persistence",
    ]
    if bare:
        command.append("--bare")
    command.append("hi")
    return command


def check_quota():
    """Start a proxy, run a minimal claude call through it, return rate-limit data."""
    server = CapturingProxy()
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    try:
        environment = {**os.environ, "ANTHROPIC_BASE_URL": server.base_url}
        result = subprocess.run(
            claude_command(bare=True),
            capture_output=True,
            text=True,
            timeout=30,
            env=environment,
        )

        # Only retry if the fast path produced nothing. `--bare` skips some
        # startup work and can exit non-zero after the request already went
        # through -- retrying on the exit code alone spends a second slice of
        # the quota we're here to measure.
        if result.returncode != 0 and not server.captured_headers:
            result = subprocess.run(
                claude_command(bare=False),
                capture_output=True,
                text=True,
                timeout=60,
                env=environment,
            )

        if not server.captured_headers:
            stderr_output = result.stderr.strip()
            stdout_output = result.stdout.strip()
            error_detail = stderr_output or stdout_output or "no output"
            raise RuntimeError(
                "No rate-limit headers captured. Claude exit code: %d. Detail: %s"
                % (result.returncode, error_detail[:200])
            )

        return server.captured_headers

    finally:
        server.shutdown()
        server.server_close()


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


def bar(percent, width=30):
    filled = int(max(0.0, min(100.0, percent)) / 100 * width)
    return f"[{'█' * filled}{'·' * (width - filled)}]"


def print_window(title, data, prefix):
    """One rolling window, if the response described it."""
    utilization = data.get("%s-utilization" % prefix)
    if not isinstance(utilization, (int, float)):
        return

    percent = utilization * 100
    print(f"  {title:<15} {bar(percent)} {percent:.1f}%")
    print(f"                  Remaining: {max(0.0, 100 - percent):.1f}%")

    reset = data.get("%s-reset" % prefix)
    if isinstance(reset, (int, float)) and reset:
        print(f"                  Resets: {format_timestamp(reset)}")
    print(f"                  Status: {data.get('%s-status' % prefix, 'unknown')}")
    print()


def print_summary(data):
    """Print a human-readable quota summary.

    Every field is optional. Anthropic has already dropped `fallback` and
    `overage-utilization` and added `overage-disabled-reason` and
    `representative-claim` since this script was written, so print what came
    back rather than asserting a fixed shape.
    """
    print()
    print("  Claude Max Quota")
    print("  " + "=" * 48)
    print()

    print_window("5-Hour Window:", data, "5h")
    print_window("7-Day Window:", data, "7d")

    overage_utilization = data.get("overage-utilization")
    if isinstance(overage_utilization, (int, float)):
        overage_percent = overage_utilization * 100
        print(f"  {'Overage:':<15} {bar(overage_percent)} {overage_percent:.1f}%")
        print(f"                  Status: {data.get('overage-status', 'unknown')}")
    else:
        print(f"  {'Overage:':<15} Status: {data.get('overage-status', 'unknown')}")

    disabled_reason = data.get("overage-disabled-reason")
    if disabled_reason:
        print(f"                  Reason: {disabled_reason}")

    fallback = data.get("fallback")
    fallback_percentage = data.get("fallback-percentage")
    if fallback and isinstance(fallback_percentage, (int, float)):
        print(f"                  Fallback: {fallback} ({fallback_percentage * 100:.0f}%)")
    elif fallback:
        print(f"                  Fallback: {fallback}")
    elif isinstance(fallback_percentage, (int, float)):
        print(f"                  Fallback: {fallback_percentage * 100:.0f}%")

    representative = data.get("representative-claim")
    if representative:
        print(f"                  Binding window: {representative}")
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
