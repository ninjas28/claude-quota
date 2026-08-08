# claude-quota

Your Claude Max quota in the macOS menu bar, plus the original CLI checker.

![menu bar ring showing session and weekly usage](https://img.shields.io/badge/macOS-13%2B-black)

## What's here

| | |
|---|---|
| **`menubar/`** | **Claude Quota Bar** — a SwiftUI menu bar app showing live usage. Start here. |
| `statusline/` | The status line bridge that feeds the app for free, and its installer |
| `claude_quota.py` | The original one-shot CLI checker (still works, see [Legacy CLI](#legacy-cli)) |

## Where the numbers come from

Anthropic has no public quota API, but there are three ways to see your rolling-window utilization. Claude Quota Bar uses the first two, in that order:

1. **The status line.** Claude Code hands its `statusLine` command a JSON blob on stdin containing `rate_limits.five_hour` and `rate_limits.seven_day` — **with no network call of its own**. The bridge script caches that to disk and the app reads it. Free, instant, and refreshes on every render while you work.
2. **The OAuth usage endpoint.** `GET https://api.anthropic.com/api/oauth/usage` — the same endpoint Claude Code's `/usage` command calls. Consumes no quota, but it's a network call against an undocumented endpoint, so the app only reaches for it when the cache has gone stale.
3. **A real API call, to read its response headers.** What `claude_quota.py` does. It works, but it costs a (tiny) slice of the quota you're measuring, so the app doesn't do this at all.

The upshot: **while a Claude Code session is open, the app makes no network calls whatsoever.** Polling only kicks in to cover the gaps.

## Install

### 1. Build the app

Requires macOS 13+ and the Xcode command line tools (`xcode-select --install`).

```bash
cd menubar
./build.sh --install     # builds, ad-hoc signs, copies to /Applications, launches
```

Drop `--install` to just build into `menubar/build/`.

### 2. Wire up the status line

This is what makes the app free to run. It's optional — the app works on the OAuth fallback alone — but recommended.

```bash
cd statusline
./install.py             # dry run: shows exactly what it will change
./install.py --apply     # writes ~/.claude/settings.json (backs it up first)
```

**Already have a status line?** It's preserved. The installer moves your existing command into `CLAUDE_QUOTA_CHAIN`, and the bridge feeds it untouched stdin and prints its output — your status line looks identical afterwards.

To undo: `./install.py --apply --remove`.

### 3. Open a Claude Code session

`rate_limits` only appears for Claude Pro/Max accounts, and only after the first API response in a session. Send one message and the ring lights up.

## Using it

The menu bar shows a colored ring and percentage: green under 50%, yellow to 80%, orange to 95%, red above. It dims when the data is going stale. Click for the full breakdown — every window Anthropic reports, with a live countdown to each reset.

**Settings** lets you pick which window drives the menu bar (session, weekly, or whichever is highest), the poll interval (1/5/10/30 min), whether to show the percentage text, whether to use the OAuth fallback at all, and launch-at-login.

## About the OAuth fallback

The fallback reads the OAuth token Claude Code already stored on your Mac and calls the usage endpoint with it. Worth knowing before you leave it on:

- It is **read-only**. The app never writes, refreshes, or rotates your credential — refreshing would rotate the refresh token out from under Claude Code and could sign you out of it. If the token has expired, the app says so and waits for Claude Code to renew it.
- It never sets `CLAUDE_CODE_OAUTH_TOKEN`, which would make Claude Code delete its Keychain entry on exit.
- The endpoint is undocumented and rate limits without warning. The app backs off exponentially on `429`, honoring `Retry-After`.
- **Anthropic's Consumer Terms state that OAuth credentials from a Claude subscription are for Claude Code and Claude.ai only, and not for use in other tools.** A local read of your own usage is a mild case, but it is your call — turn the fallback off in Settings and the app runs purely on the status line cache, which is entirely above board.

## Legacy CLI

`claude_quota.py` predates all of this. It starts a local proxy, routes one cheap Haiku call through it, and reads the `anthropic-ratelimit-unified-*` response headers. Still useful for scripting:

```bash
python3 claude_quota.py           # pretty summary
python3 claude_quota.py --json    # machine-readable
```

It reports two things the newer sources don't: `overage-utilization` and `fallback`. But every check consumes a little quota, so prefer the app for anything continuous.

## Architecture

```
Claude Code session
  └─ statusLine command
       └─ claude-quota-statusline.py     writes ~/.claude/quota-bar-cache.json
            │                             (atomic: temp file + rename)
            ▼
       Claude Quota Bar
            ├─ CacheFileWatcher     picks up writes instantly
            ├─ 5-min timer          re-reads cache; polls only if stale
            └─ OAuthUsageClient     GET /api/oauth/usage, backs off on 429
```

The bridge script never raises: a status line that throws would show an error in Claude Code on every render, and caching quota isn't worth that. If `rate_limits` is missing from a payload it leaves the previous cache untouched rather than clobbering good data with nothing.

## Limitations

- `rate_limits` requires a Claude.ai Pro or Max subscription. API-key users get nothing from either source.
- The cache goes stale when Claude Code is closed. That's what the OAuth fallback is for; without it the app shows the last known values, dimmed.
- Both sources are undocumented or best-effort. Anthropic can change them.
- The app is ad-hoc signed, not notarized. macOS may ask you to confirm the first launch.

## License

MIT
