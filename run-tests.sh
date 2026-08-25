#!/usr/bin/env bash
#
# Everything that can fail, cheapest failure first.
#
#   ./run-tests.sh
#
# The Swift build runs first on purpose. The app once shipped in a state that
# would not compile at all while the Python suite stayed green, because nothing
# in the test path ever built it. The Rust build is here for the same reason.
#
# Each app is required on the platform that can build it and skipped on the one
# that cannot, so this is runnable from either side. The Python suite is the
# only part that always runs -- it tests the status line bridge, which both
# apps read.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO"

case "$(uname -s)" in
    Darwin*) HOST=macos ;;
    MINGW* | MSYS* | CYGWIN*) HOST=windows ;;
    *) HOST=other ;;
esac

if [[ "$HOST" == macos ]]; then
    if ! command -v swift >/dev/null 2>&1; then
        echo "error: swift not found. Install the Xcode command line tools:" >&2
        echo "         xcode-select --install" >&2
        exit 1
    fi

    echo "==> Building the menu bar app (release)"
    (cd menubar && swift build -c release --disable-sandbox)

    echo
    echo "==> Swift tests"
    (cd menubar && swift test)
    echo
else
    echo "==> Skipping the macOS app (no swift on this platform)"
    echo
fi

if [[ "$HOST" == windows ]] && ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install Rust from https://rustup.rs" >&2
    exit 1
fi

if command -v cargo >/dev/null 2>&1; then
    echo "==> Rust tests (Windows app)"
    (cd windows && cargo test)
    echo
else
    echo "==> Skipping the Windows app (no cargo on this platform)"
    echo
fi

if [[ "$HOST" == windows ]]; then
    # The Python suite assumes a POSIX shell: it chains to .sh scripts, makes
    # directories unwritable with chmod, and decodes the bridge's output with
    # the locale codepage rather than UTF-8. Those are the tests being
    # POSIX-only, not the bridge -- which does work on Windows, and whose cache
    # format is covered from the other side by the Rust suite above.
    echo "==> Skipping the Python tests (the suite assumes a POSIX shell)"
    echo
else
    echo "==> Python tests"
    python3 -m unittest discover -s tests
fi

echo
echo "==> All green"
