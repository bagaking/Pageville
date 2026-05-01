#!/usr/bin/env bash
set -euo pipefail

# Two data-面 regressions that fail silently rather than loudly:
#   - since/until compared as strings, so an equivalent RFC3339 spelling
#     ("...Z" vs "+00:00") silently returned a different set of events;
#   - a published file or directory whose name looks like a snapshot id
#     (12 hex chars — normal for hashed assets) was shadowed by the version
#     route and became permanently unreachable.

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17787}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/site/abcdef012345"
printf 'ROOT-INDEX\n' > "$tmp_dir/site/index.html"
printf 'HEX-DIR-ASSET\n' > "$tmp_dir/site/abcdef012345/style.css"
printf 'HEX-ROOT-ASSET\n' > "$tmp_dir/site/0123456789ab"

run() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- "$@"; }
id="$(run publish "$tmp_dir/site" --page hexed | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
test -n "$id"
base="http://127.0.0.1:${port}"

# Hex-named content is reachable; the real snapshot id still pins a version.
test "$(curl -fsS "$base/hexed/abcdef012345/style.css")" = 'HEX-DIR-ASSET'
test "$(curl -fsS "$base/hexed/0123456789ab")" = 'HEX-ROOT-ASSET'
test "$(curl -fsS "$base/hexed/${id}/")" = 'ROOT-INDEX'
test "$(curl -fsS "$base/hexed/latest/")" = 'ROOT-INDEX'
# A hex-shaped segment that is neither a real file nor a real snapshot: 404.
test "$(curl -sS -o /dev/null -w '%{http_code}' "$base/hexed/ffffffffffff/")" = 404
# api/v0/context agrees with the routing.
test "$(curl -fsS "$base/api/v0/context?path=/hexed/${id}/" | jq -r .version)" = "$id"

# Timestamp filtering must compare instants, not bytes.
ev="$(curl -fsS -X POST "$base/api/v0/pages/hexed/events" -H 'content-type: application/json' \
  -d "{\"version\":\"$id\",\"session\":\"s1\",\"payload\":{\"kind\":\"probe\"}}")"
ts="$(printf '%s' "$ev" | jq -r .ts)"
bare="${ts%%.*}"

# All three spellings denote the same instant, so all three must return the event.
for form in "${bare}Z" "${bare}+00:00" "$bare"; do
  n="$(run events pull --page hexed --since "$form" | grep -c . || true)"
  test "$n" -eq 1 || { echo "FAIL: --since $form returned $n events, expected 1" >&2; exit 1; }
done

# A bound one hour AFTER the stored instant, spelled in -08:00 so that it sorts
# BEFORE the stored stamp as a string. Comparing instants includes the event;
# comparing bytes drops it. Python does the arithmetic because shell hour maths
# silently rolls past midnight without rolling the date.
future="$(python3 -c '
import datetime, sys
utc = datetime.timezone.utc
t = datetime.datetime.fromisoformat(sys.argv[1]).replace(tzinfo=utc) + datetime.timedelta(hours=1)
print(t.astimezone(datetime.timezone(datetime.timedelta(hours=-8))).isoformat())
' "$bare")"
# Guard against a vacuous assertion: if the bound ever stops sorting before the
# stored stamp, the test would pass under the string-compare bug it exists to catch.
test "$future" \< "$ts" || { echo "FAIL: bound $future no longer sorts before $ts; test is vacuous" >&2; exit 1; }
n="$(run events pull --page hexed --until "$future" | grep -c . || true)"
test "$n" -eq 1 || { echo "FAIL: --until $future returned $n events, expected 1 (instant is later than $ts)" >&2; exit 1; }

# An unparseable bound must be rejected, not silently filtered. Answering "no
# events" for a typo is a wrong answer a caller cannot distinguish from a
# genuinely empty range.
for bad in notatime zzzz ''; do
  code="$(curl -sS -o /dev/null -w '%{http_code}' "$base/api/v0/pages/hexed/events?since=$bad")"
  test "$code" = 400 || { echo "FAIL: since=$bad returned $code, expected 400" >&2; exit 1; }
  code="$(curl -sS -o /dev/null -w '%{http_code}' "$base/api/v0/pages/hexed/events?until=$bad")"
  test "$code" = 400 || { echo "FAIL: until=$bad returned $code, expected 400" >&2; exit 1; }
done
# The CLI must surface that rejection instead of printing the error body as if
# it were the event stream and exiting 0.
if out="$(run events pull --page hexed --since notatime 2>/dev/null)"; then
  echo "FAIL: CLI exited 0 for an invalid --since (printed: $out)" >&2; exit 1
fi
test -z "$out" || { echo "FAIL: CLI wrote '$out' to stdout for a rejected query" >&2; exit 1; }

echo 'PASS: hex-named assets stay reachable and since/until compare instants across RFC3339 spellings'
