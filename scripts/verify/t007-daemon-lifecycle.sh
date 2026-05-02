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
test "$(run daemon status | head -1)" = running
test "$(run --json daemon status | jq -r .protocol)" = v0
daemon_pid="$(tr -d '[:space:]' < "$tmp_dir/data/daemon.pid")"
test -n "$daemon_pid"
kill -0 "$daemon_pid"
ps -p "$daemon_pid" -o command= | grep -q 'pageville.*daemon run'
run daemon stop
for _ in $(seq 1 30); do test "$(run daemon status | head -1)" = stopped && break || sleep 0.1; done
test "$(run daemon status | head -1)" = stopped
# A lock left behind by a crashed start must not block the next one. Writing
# junk into daemon.lock would prove nothing — the lock is an OS advisory lock,
# so its file CONTENT is irrelevant. Take the lock in a real process and SIGKILL
# it, which is what a crash actually leaves behind.
printf 'stale-marker\n' > "$tmp_dir/data/daemon.lock"
printf '999999\n' > "$tmp_dir/data/daemon.pid"
python3 -c "
import fcntl, sys, time
f = open('$tmp_dir/data/daemon.lock', 'r+')
fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
sys.stderr.write('locked\n'); sys.stderr.flush()
time.sleep(60)
" 2>"$tmp_dir/lockproc.err" &
lock_pid=$!
for _ in $(seq 1 50); do grep -q locked "$tmp_dir/lockproc.err" 2>/dev/null && break || sleep 0.1; done
grep -q locked "$tmp_dir/lockproc.err"
kill -9 "$lock_pid" 2>/dev/null || true
wait "$lock_pid" 2>/dev/null || true
# The holder died without releasing; the next start must still succeed.
run pages list >/dev/null
test "$(run daemon status | head -1)" = running
run daemon stop
for _ in $(seq 1 30); do test "$(run daemon status | head -1)" = stopped && break || sleep 0.1; done
test "$(run daemon status | head -1)" = stopped
run publish "$tmp_dir/site" --page restart-demo >/dev/null
test "$(run daemon status | head -1)" = running

# PAGEVILLE_PORT=0 parses as a valid u16 but means "any free port" to the OS:
# the child bound something random while the parent probed :0 forever, hanging
# the CLI unboundedly and orphaning a live daemon on every invocation. It must
# now fall back to the default port and terminate promptly.
#
# The fallback target is the default 7777, so this check is skipped (loudly,
# never silently) when something already listens there — publishing into a
# developer's real daemon would be worse than losing one assertion. The pure
# fallback rule itself is covered by the `port_zero_falls_back` unit test.
if curl -fsS -m 2 "http://127.0.0.1:7777/api/v0/health" >/dev/null 2>&1; then
  echo '  (skipping PAGEVILLE_PORT=0 check: a daemon already holds 7777)'
else
  zero_dir="$tmp_dir/zero"
  mkdir -p "$zero_dir/site"
  printf 'ZERO\n' > "$zero_dir/site/index.html"
  PAGEVILLE_DATA_DIR="$zero_dir/data" PAGEVILLE_PORT=0 \
    cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- \
    publish "$zero_dir/site" --page zeroport >/dev/null 2>&1 &
  zero_pid=$!
  waited=0
  while kill -0 "$zero_pid" 2>/dev/null && [ "$waited" -lt 60 ]; do
    sleep 1
    waited=$((waited + 1))
  done
  if kill -0 "$zero_pid" 2>/dev/null; then
    kill -9 "$zero_pid" 2>/dev/null || true
    PAGEVILLE_DATA_DIR="$zero_dir/data" \
      cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true
    echo 'FAIL: PAGEVILLE_PORT=0 hung the CLI instead of falling back' >&2
    exit 1
  fi
  wait "$zero_pid" 2>/dev/null || true
  PAGEVILLE_DATA_DIR="$zero_dir/data" \
    cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true
fi

# All growth is immutable snapshot history that no GC may reclaim, so the
# honest intervention is visibility: `daemon status` must report the real
# on-disk size and it must track actual growth.
size_dir="$tmp_dir/size"
mkdir -p "$size_dir/site"
python3 -c "open('$size_dir/site/big.bin','w').write('A'*200000)"
printf 'x\n' > "$size_dir/site/index.html"
size_run() { PAGEVILLE_DATA_DIR="$size_dir/data" PAGEVILLE_PORT="$port" \
  cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- "$@"; }
size_run publish "$size_dir/site" --page sized >/dev/null
# Line 1 stays the bare predicate word; the detail line sits under it.
test "$(size_run daemon status | head -1)" = running
size_run daemon status | sed -n 2p | grep -q '^data_dir=.* objects=[0-9]* snapshots=[0-9]* size=' \
  || { echo 'FAIL: daemon status lost its size detail line' >&2; exit 1; }
b1="$(size_run --json daemon status | python3 -c 'import sys,json;print(json.load(sys.stdin)["object_bytes"])')"
test "$b1" -ge 200000 || { echo "FAIL: status reported $b1 bytes, expected >=200000" >&2; exit 1; }
python3 -c "open('$size_dir/site/big.bin','w').write('B'*200000)"
size_run publish "$size_dir/site" --page sized >/dev/null
b2="$(size_run --json daemon status | python3 -c 'import sys,json;print(json.load(sys.stdin)["object_bytes"])')"
test "$b2" -gt "$b1" || { echo "FAIL: object_bytes did not grow ($b1 -> $b2)" >&2; exit 1; }
size_run daemon stop >/dev/null 2>&1 || true

echo 'PASS: idempotent auto-start, health protocol, clean stop, and restart'
