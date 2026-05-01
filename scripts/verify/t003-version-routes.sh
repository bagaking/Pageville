#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17779}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/site"
publish() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$tmp_dir/site" --page demo; }
printf 'version-one\n' > "$tmp_dir/site/index.html"
id1="$(publish | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
printf 'version-two\n' > "$tmp_dir/site/index.html"
id2="$(publish | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
test "$id1" != "$id2"
test "$(curl -fsS "http://127.0.0.1:${port}/demo/")" = 'version-two'
test "$(curl -fsS "http://127.0.0.1:${port}/demo/latest/")" = 'version-two'
test "$(curl -fsS "http://127.0.0.1:${port}/demo/${id2}/")" = 'version-two'
test "$(curl -fsS "http://127.0.0.1:${port}/demo/${id1}/")" = 'version-one'
if PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$tmp_dir/site" --page 'Bad_Name' >/dev/null 2>&1; then exit 1; fi
if PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$tmp_dir/site" --page api >/dev/null 2>&1; then exit 1; fi

# PRD §9 #3: reads concurrent with a publish must see either the whole old
# snapshot or the whole new one — never a 404 and never mixed content. A
# sequential loop after both publishes finish proves nothing about this, so
# keep publishing in the background while hammering the route.
(
  for i in $(seq 1 10); do
    printf 'concurrent-%d\n' "$i" > "$tmp_dir/site/index.html"
    PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$tmp_dir/site" --page demo >/dev/null 2>&1
  done
) &
publisher=$!
reads=0
while kill -0 "$publisher" 2>/dev/null; do
  body="$(curl -fsS "http://127.0.0.1:${port}/demo/")" || { echo "FAIL: read errored during publish" >&2; exit 1; }
  printf '%s' "$body" | grep -Eq '^(version-two|concurrent-[0-9]+)$' || { echo "FAIL: torn read: $body" >&2; exit 1; }
  reads=$((reads + 1))
done
wait "$publisher"
test "$reads" -gt 0
echo "  (${reads} reads served cleanly while publishes were in flight)"
echo 'PASS: pinned versions, latest aliases, slug validation, and atomic latest under concurrent publishes'
