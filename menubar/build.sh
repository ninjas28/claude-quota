#!/usr/bin/env bash
#
# Builds Claude Quota Bar and assembles it into a .app bundle.
#
#   ./build.sh              build into ./build/Claude Quota Bar.app
#   ./build.sh --install    also copy it into /Applications and launch it
#
# Requires Xcode command line tools (`xcode-select --install`) and macOS 13+.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

APP_NAME="Claude Quota Bar"
BUILD_DIR="$SCRIPT_DIR/build"
APP_BUNDLE="$BUILD_DIR/$APP_NAME.app"

if ! command -v swift >/dev/null 2>&1; then
    echo "error: swift not found. Install the Xcode command line tools:" >&2
    echo "         xcode-select --install" >&2
    exit 1
fi

echo "==> Building (release)"
swift build -c release --disable-sandbox

BINARY="$(swift build -c release --show-bin-path)/ClaudeQuotaBar"
if [[ ! -x "$BINARY" ]]; then
    echo "error: build succeeded but no binary at $BINARY" >&2
    exit 1
fi

echo "==> Assembling $APP_NAME.app"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

cp "$BINARY" "$APP_BUNDLE/Contents/MacOS/ClaudeQuotaBar"
cp "$SCRIPT_DIR/Resources/Info.plist" "$APP_BUNDLE/Contents/Info.plist"
printf 'APPL????' > "$APP_BUNDLE/Contents/PkgInfo"

# Ad-hoc signature. Unsigned menu bar apps get killed by Gatekeeper on launch,
# and SMAppService (launch at login) refuses to register without any signature.
echo "==> Signing (ad-hoc)"
codesign --force --deep --sign - "$APP_BUNDLE"

echo "==> Built: $APP_BUNDLE"

if [[ "${1:-}" == "--install" ]]; then
    echo "==> Installing to /Applications"
    rm -rf "/Applications/$APP_NAME.app"
    cp -R "$APP_BUNDLE" "/Applications/$APP_NAME.app"
    open "/Applications/$APP_NAME.app"
    echo "==> Launched. Look for the ring in your menu bar."
fi
