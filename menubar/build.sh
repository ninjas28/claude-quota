#!/usr/bin/env bash
#
# Builds Claude Quota Bar into ./build/Claude Quota Bar.app.
#
# Installing is ../install.sh's job. This only ever builds.

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
# No --deep: Apple deprecated it, and there is nothing nested to sign here —
# the bundle is a single binary.
echo "==> Signing (ad-hoc)"
codesign --force --sign - "$APP_BUNDLE"

echo "==> Built: $APP_BUNDLE"
