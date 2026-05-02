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

# A rejected publish must write NO objects. Validating and writing in one pass
# stored the files that passed before bailing on a later bad one, orphaning them
# permanently: no snapshot row is committed, and snapshots are the only
# reachability root, so nothing ever reclaims them.
before="$(find "$tmp_dir/data/objects" -type f | wc -l | tr -d ' ')"
for bad_key in 'zzz/../evil.bin' 'zzz-bad-b64.bin'; do
  if [ "$bad_key" = 'zzz-bad-b64.bin' ]; then bad_val='!!!not-base64!!!'; else bad_val='RQ=='; fi
  # Distinct payloads so content-addressing cannot dedupe them into one object.
  python3 -c "
import base64, json, sys, urllib.request, urllib.error
files = {'orphan%d.bin' % i: base64.b64encode(bytes([65+i]) * 50000).decode() for i in range(4)}
files[sys.argv[1]] = sys.argv[2]
body = json.dumps({'files': files, 'spa': False}).encode()
req = urllib.request.Request(sys.argv[3], data=body, headers={'Content-Type': 'application/json'})
try:
    urllib.request.urlopen(req, timeout=30)
    print('FAIL: bad publish was accepted', file=sys.stderr); sys.exit(1)
except urllib.error.HTTPError as e:
    sys.exit(0 if e.code == 400 else 1)
" "$bad_key" "$bad_val" "http://127.0.0.1:${port}/api/v0/pages/demo/snapshots"
done
after="$(find "$tmp_dir/data/objects" -type f | wc -l | tr -d ' ')"
test "$after" -eq "$before" || {
  echo "FAIL: rejected publishes orphaned $((after - before)) objects" >&2; exit 1; }

echo 'PASS: CAS objects, snapshot manifests, and content-derived snapshot ids are idempotent'
