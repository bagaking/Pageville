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
daemon_pid="$(tr -d '[:space:]' < "$tmp_dir/data/daemon.pid")"
test -n "$daemon_pid"
kill -0 "$daemon_pid"
ps -p "$daemon_pid" -o command= | grep -q 'pageville.*daemon run'
run daemon stop
for _ in $(seq 1 30); do test "$(run daemon status)" = stopped && break || sleep 0.1; done
test "$(run daemon status)" = stopped
printf 'stale-marker\n' > "$tmp_dir/data/daemon.lock"
printf '999999\n' > "$tmp_dir/data/daemon.pid"
run pages list >/dev/null
test "$(run daemon status)" = running
run daemon stop
for _ in $(seq 1 30); do test "$(run daemon status)" = stopped && break || sleep 0.1; done
test "$(run daemon status)" = stopped
run publish "$tmp_dir/site" --page restart-demo >/dev/null
test "$(run daemon status)" = running
echo 'PASS: idempotent auto-start, health protocol, clean stop, and restart'
