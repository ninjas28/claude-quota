#!/usr/bin/env bash
#
# Claude Quota Bar installer.
#
# Builds from source rather than shipping a disk image: the app is ad-hoc
# signed, and a downloaded .dmg would be quarantined and refused by Gatekeeper.
# A locally built bundle isn't.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_NAME="Claude Quota Bar"
APP_DIR="${APP_DIR:-/Applications}"
TARGET="$APP_DIR/$APP_NAME.app"
BUNDLE_ID="com.claudequota.ClaudeQuotaBar"

ASSUME_YES=0
WITH_STATUSLINE=""
LAUNCH=1
UNINSTALL=0
KEEP_SETTINGS=0

usage() {
    cat <<'EOF'
Claude Quota Bar installer

  ./install.sh                   build, install, offer to set up the status line
  ./install.sh --yes             accept every prompt
  ./install.sh --no-statusline   install the app only
  ./install.sh --uninstall       remove everything this installed

Options
  -y, --yes          don't prompt
      --statusline   set up the status line without asking
      --no-launch    don't open the app afterwards
      --keep-settings  on uninstall, leave preferences in place
      --uninstall    remove the app, restore the status line, clear preferences

Environment
  APP_DIR   where the app is installed (default /Applications)
EOF
}

for argument in "$@"; do
    case "$argument" in
        -y|--yes) ASSUME_YES=1 ;;
        --statusline) WITH_STATUSLINE=1 ;;
        --no-statusline) WITH_STATUSLINE=0 ;;
        --no-launch) LAUNCH=0 ;;
        --keep-settings) KEEP_SETTINGS=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'unknown option: %s\n\n' "$argument" >&2; usage >&2; exit 2 ;;
    esac
done

say()  { printf '\033[1m==>\033[0m %s\n' "$1"; }
step() { printf '    %s\n' "$1"; }
die()  { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }

# Yes unless explicitly declined. Non-interactive shells decline, so piping
# this script somewhere can never silently edit settings.json.
ask() {
    [[ $ASSUME_YES == 1 ]] && return 0
    [[ -t 0 ]] || return 1
    local answer
    read -r -p "    $1 [Y/n] " answer
    [[ -z "$answer" || "$answer" =~ ^[Yy] ]]
}

quit_running() {
    pkill -f "$APP_NAME.app/Contents/MacOS/ClaudeQuotaBar" 2>/dev/null || true
}

if [[ $UNINSTALL == 1 ]]; then
    say "Removing $APP_NAME"
    quit_running
    if [[ -d "$TARGET" ]]; then
        rm -rf "$TARGET"
        step "removed $TARGET"
    else
        step "not installed at $TARGET"
    fi

    say "Restoring the status line"
    "$SCRIPT_DIR/statusline/install.py" --apply --remove 2>&1 | sed 's/^/    /' || true

    if [[ $KEEP_SETTINGS == 0 ]]; then
        say "Clearing preferences and cache"
        defaults delete "$BUNDLE_ID" 2>/dev/null && step "cleared preferences" \
            || step "no preferences to clear"
        rm -f "$HOME/.claude/quota-bar-cache.json"
    fi

    say "Done."
    exit 0
fi

say "Checking requirements"
[[ "$(uname -s)" == "Darwin" ]] || die "macOS only."
macos_version="$(sw_vers -productVersion)"
[[ "${macos_version%%.*}" -ge 13 ]] || die "needs macOS 13 or later (found $macos_version)."
command -v swift >/dev/null 2>&1 || die "swift not found. Run: xcode-select --install"
step "macOS $macos_version"
step "$(swift --version 2>/dev/null | head -1)"

say "Building"
"$SCRIPT_DIR/menubar/build.sh" 2>&1 | sed 's/^/    /'

BUILT="$SCRIPT_DIR/menubar/build/$APP_NAME.app"
[[ -d "$BUILT" ]] || die "build finished but no bundle at $BUILT"

say "Installing to $APP_DIR"
mkdir -p "$APP_DIR"
quit_running
rm -rf "$TARGET"
cp -R "$BUILT" "$TARGET"
codesign --verify "$TARGET" 2>/dev/null || die "the installed bundle failed signature verification"
step "$TARGET"

if [[ -z "$WITH_STATUSLINE" ]]; then
    say "Status line bridge (optional)"
    step "Claude Code hands its status line your usage on every turn, for free."
    step "Hooking into it means the app needs no network calls while you work."
    step "This edits ~/.claude/settings.json, backing it up first. Any status"
    step "line you already have keeps working."
    if ask "Set it up?"; then WITH_STATUSLINE=1; else WITH_STATUSLINE=0; fi
fi

if [[ $WITH_STATUSLINE == 1 ]]; then
    say "Setting up the status line"
    "$SCRIPT_DIR/statusline/install.py" --apply 2>&1 | sed 's/^/    /'
fi

if [[ $LAUNCH == 1 ]]; then
    say "Launching"
    open "$TARGET"
fi

say "Done."
if [[ $WITH_STATUSLINE == 1 ]]; then
    step "Send a message in Claude Code and the indicator will fill in."
else
    step "Send a message in Claude Code, or turn on the usage API in Settings."
fi
step "Uninstall with: $0 --uninstall"
