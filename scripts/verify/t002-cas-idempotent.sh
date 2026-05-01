#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17778}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/site"
printf 'same-content\n' > "$tmp_dir/site/index.html"

publish() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$tmp_dir/site" --page "$1"; }
id1="$(publish demo | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
id2="$(publish demo | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
test -n "$id1" && test "$id1" = "$id2"
count_before="$(find "$tmp_dir/data/objects" -type f | wc -l | tr -d ' ')"
id_other="$(publish other | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
test -n "$id_other" && test "$id_other" != "$id1"
test "$(curl -fsS "http://127.0.0.1:${port}/other/")" = 'same-content'
printf 'changed-content\n' > "$tmp_dir/site/index.html"
id3="$(publish demo | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
test "$id3" != "$id1"
count_after="$(find "$tmp_dir/data/objects" -type f | wc -l | tr -d ' ')"
test "$count_after" -gt "$count_before"
# The manifest lives in SQLite (snapshots.manifest) and is read back from
# there; assert the snapshot resolves rather than that a redundant file exists.
test "$(curl -fsS "http://127.0.0.1:${port}/demo/${id1}/")" = 'same-content'
test "$(curl -fsS "http://127.0.0.1:${port}/demo/${id3}/")" = 'changed-content'

# A crash mid-write leaves a truncated object. Republishing must repair it:
# skipping on mere existence would serve half a page forever.
obj="$(grep -rl 'changed-content' "$tmp_dir/data/objects" | head -1)"
test -n "$obj" || { echo 'FAIL: could not locate the CAS object' >&2; exit 1; }
full="$(wc -c < "$obj" | tr -d ' ')"
python3 -c "
import sys
p = sys.argv[1]
b = open(p, 'rb').read()
open(p, 'wb').write(b[:len(b)//2])" "$obj"
test "$(wc -c < "$obj" | tr -d ' ')" -lt "$full"
publish demo >/dev/null
test "$(wc -c < "$obj" | tr -d ' ')" -eq "$full" || { echo "FAIL: truncated object not repaired by republish" >&2; exit 1; }
test "$(curl -fsS "http://127.0.0.1:${port}/demo/")" = 'changed-content'
test "$(find "$tmp_dir/data/objects" -name '*.tmp' | wc -l | tr -d ' ')" -eq 0

echo 'PASS: CAS objects, snapshot manifests, and content-derived snapshot ids are idempotent'
