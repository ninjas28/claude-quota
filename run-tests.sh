#!/usr/bin/env bash
#
# Everything that can fail, cheapest failure first.
#
#   ./run-tests.sh
#
# The Swift build runs first on purpose. The app once shipped in a state that
# would not compile at all while the Python suite stayed green, because nothing
# in the test path ever built it.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO"

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
echo "==> Python tests"
python3 -m unittest discover -s tests

echo
echo "==> All green"
