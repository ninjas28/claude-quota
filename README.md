# claude-quota

Check your Claude Max subscription quota from the command line.

Anthropic doesn't expose a public API for checking how much of your Claude Max quota you've used. But every API response includes undocumented `anthropic-ratelimit-unified-*` headers with your exact utilization percentages. This tool captures those headers by routing a single cheap Claude Code CLI call through a local proxy.

## Example output

```
  Claude Max Quota
  ================================================

  5-Hour Window:  [████████████████████████████··] 93.0%
                  Remaining: 7.0%
                  Resets: Sun Mar 29 03:00 AM (in 4.3 hrs)
                  Status: allowed_warning

  7-Day Window:   [█████·························] 18.0%
                  Remaining: 82.0%
                  Resets: Sat Apr 04 12:00 PM (in 6.6 days)
                  Status: allowed

  Overage:        [······························] 0.0%
                  Status: allowed
                  Fallback: available (50%)
```

## Requirements

- **Claude Code CLI** (`claude`) installed and logged in
- **Python 3.7+** (no third-party dependencies -- uses only the standard library)
- **macOS** (Claude Code uses the macOS Keychain for auth; Linux support may work if Claude Code handles auth differently there)

## Usage

```bash
# Pretty-printed summary
python3 claude_quota.py

# Machine-readable JSON
python3 claude_quota.py --json
```

### JSON output

```json
{
  "status": "allowed_warning",
  "5h-status": "allowed_warning",
  "5h-reset": 1774778400.0,
  "5h-utilization": 0.97,
  "7d-status": "allowed",
  "7d-reset": 1775329200.0,
  "7d-utilization": 0.18,
  "overage-status": "allowed",
  "overage-reset": 1775001600.0,
  "overage-utilization": 0.0,
  "fallback-percentage": 0.5,
  "fallback": "available",
  "reset": 1774778400.0
}
```

### Key fields

| Field | Description |
|-------|-------------|
| `5h-utilization` | 0.0 -- 1.0, how much of your rolling 5-hour window you've consumed |
| `7d-utilization` | 0.0 -- 1.0, how much of your rolling 7-day window you've consumed |
| `5h-reset` / `7d-reset` | Unix timestamps for when each window resets |
| `status` | `allowed`, `allowed_warning`, or `limited` |
| `fallback` | Whether overage/fallback capacity is `available` or `exhausted` |
| `overage-utilization` | How much of your overage allocation you've used |

## How it works

1. Starts a tiny HTTP proxy on a random local port
2. Runs `claude -p --model haiku "hi"` routed through the proxy via `ANTHROPIC_BASE_URL`
3. The proxy forwards the request to `api.anthropic.com` over HTTPS
4. Captures the `anthropic-ratelimit-unified-*` response headers
5. Parses and displays the utilization data

The cost per check is effectively zero -- a single Haiku call with a 2-token prompt.

## Use cases

- **End-of-day quota burning**: Check remaining quota, then fire off requests to use up what's left
- **CI/automation gating**: Skip expensive Claude calls if quota is low
- **Personal dashboards**: Pipe `--json` into whatever monitoring you like
- **Cron alerts**: Get notified when you're about to hit limits

## Limitations

- Relies on undocumented response headers. Anthropic could change these at any time.
- Each check makes one real (tiny) API call, which does consume a small amount of quota.
- Requires Claude Code CLI to be installed and authenticated -- this is not a standalone API client.

## License

MIT
