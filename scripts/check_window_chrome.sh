#!/usr/bin/env bash
# AppKit requires a macOS WindowServer; keep this with the window/GPU smoke checks.
set -euo pipefail
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_dir"
mkdir -p build/checks
chrome_dir="$(mktemp -d "$repo_dir/build/checks/window-chrome-XXXXXXXX")"
xcrun clang -fobjc-arc -fmodules -mmacosx-version-min=12.0 -Wall -Wextra -Werror \
    -framework AppKit tests/native/window_chrome.m native/window_chrome.m \
    -o "$chrome_dir/window-chrome"
"$chrome_dir/window-chrome"
