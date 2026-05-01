#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17783}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
run() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- "$@"; }
mkdir -p "$tmp_dir/site"
printf 'lifecycle\n' > "$tmp_dir/site/index.html"
for _ in $(seq 1 8); do run pages list >/dev/null & done
wait
test "$(run daemon status)" = running
test "$(run --json daemon status | jq -r .protocol)" = v0
pid_count="$(pgrep -f "pageville.*daemon run" | wc -l | tr -d ' ')"
test "$pid_count" -eq 1
run daemon stop
for _ in $(seq 1 30); do test "$(run daemon status)" = stopped && break || sleep 0.1; done
test "$(run daemon status)" = stopped
run publish "$tmp_dir/site" --page restart-demo >/dev/null
test "$(run daemon status)" = running
echo 'PASS: idempotent auto-start, health protocol, clean stop, and restart'
