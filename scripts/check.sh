#!/usr/bin/env bash
# Every requested check runs; logs retain failures and explicit GPU skips.
set -u
set -o pipefail
export CARGO_INCREMENTAL=0
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_dir" || exit 1
mode="${1:-all}"
case "$mode" in all|core|gpu|smoke) ;; *) echo "Usage: bash scripts/check.sh [all|core|gpu|smoke]" >&2; exit 2 ;; esac
mkdir -p build/checks || exit 1
log_dir="$(mktemp -d "$repo_dir/build/checks/run-XXXXXXXX")" || exit 1
{ date -u; rustc --version; cargo --version; uname -a; git rev-parse HEAD; git status --short; } > "$log_dir/environment.log" 2>&1
failed=0
run_check() {
    local name="$1"
    shift
    echo "Running $name (log: $log_dir/$name.log)"
    "$@" > "$log_dir/$name.log" 2>&1
    local result=$?
    echo "$result" > "$log_dir/$name.exit"
    if [ "$result" -ne 0 ]; then failed=1; echo "$name: FAILED ($result)"; tail -n 30 "$log_dir/$name.log"
    else echo "$name: completed (inspect logs for skips)"; fi
}
if [ "$mode" = all ] || [ "$mode" = core ]; then
    run_check fmt cargo fmt --all -- --check
    run_check core cargo test --release --locked --no-default-features -- --nocapture
    run_check alloc cargo test --release --locked --features alloc -- --nocapture
fi
if [ "$mode" = all ]; then
    run_check text cargo test --release --locked --features text --lib -- --nocapture
    run_check portable cargo test --release --locked --features portable --all-targets -- --nocapture
    run_check clippy cargo clippy --release --locked --features portable --all-targets -- -D warnings
    run_check doctest cargo test --release --locked --features portable --doc
    run_check docs env RUSTDOCFLAGS="-D warnings" cargo doc --release --locked --no-deps --features portable --lib
    run_check performance cargo run --release --locked --features alloc --example performance
    run_check package cargo package --locked --allow-dirty --no-verify
fi
if [ "$mode" = gpu ]; then
    export LAGUI_REQUIRE_GPU=1
    run_check gpu cargo test --release --locked --features portable --lib --tests -- --nocapture
fi
if [ "$mode" = smoke ]; then
    if [ "$(uname -s)" = Darwin ]; then run_check window-chrome bash scripts/check_window_chrome.sh; fi
    run_check studio cargo run --release --locked --features portable --bin studio-demo -- --smoke
    run_check basic cargo run --release --locked --features portable --bin portable-demo -- --smoke
    if [ "$(uname -s)" = Darwin ]; then run_check native cargo run --release --locked --features native --bin native-demo -- --smoke; fi
fi
echo "Logs: $log_dir"
exit "$failed"
