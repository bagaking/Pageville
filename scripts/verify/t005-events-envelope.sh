#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17781}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/site"
printf 'event-page-v1\n' > "$tmp_dir/site/index.html"
publish() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$tmp_dir/site" --page demo; }
id1="$(publish | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
printf 'event-page-v2\n' > "$tmp_dir/site/index.html"
id2="$(publish | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"

headers="$(curl -fsSI "http://127.0.0.1:${port}/demo/latest/")"
printf '%s\n' "$headers" | grep -qi "x-pageville-page: demo"
printf '%s\n' "$headers" | grep -qi "x-pageville-version: ${id2}"
test "$(curl -fsS "http://127.0.0.1:${port}/api/v0/context?path=/demo/deep/link" | jq -r .version)" = "$id2"

post() { curl -fsS -X POST "http://127.0.0.1:${port}/api/v0/pages/demo/events" -H 'content-type: application/json' -d "$1"; }
ev1="$(post "{\"version\":\"$id1\",\"session\":\"s1\",\"payload\":{\"kind\":\"old\"}}")"
ev2="$(post "{\"version\":\"$id2\",\"session\":\"s2\",\"payload\":{\"kind\":\"new\"}}")"
test "$(printf '%s' "$ev1" | jq -r .version)" = "$id1"
test "$(printf '%s' "$ev1" | jq -r .event_id)" != null
test "$(printf '%s' "$ev1" | jq -r .ts)" != null
if post '{"version":"latest","session":"bad","payload":{}}' >/dev/null 2>&1; then exit 1; fi
if post '{"version":"doesnotexist","session":"bad","payload":{}}' >/dev/null 2>&1; then exit 1; fi

all="$(curl -fsS "http://127.0.0.1:${port}/api/v0/pages/demo/events")"
test "$(printf '%s\n' "$all" | wc -l | tr -d ' ')" -ge 2
test "$(curl -fsS "http://127.0.0.1:${port}/api/v0/pages/demo/events?session=s1" | jq -r .payload.kind)" = old
test "$(curl -fsS "http://127.0.0.1:${port}/api/v0/pages/demo/events?version=${id2}" | jq -r .payload.kind)" = new
since="$(printf '%s' "$ev2" | jq -r .ts)"
encoded_since="${since//+/%2B}"
test "$(curl -fsS "http://127.0.0.1:${port}/api/v0/pages/demo/events?since=${encoded_since}&until=${encoded_since}" | jq -r .payload.kind)" = new
special_session='session&part=+1'
ev3="$(post "{\"version\":\"$id2\",\"session\":\"$special_session\",\"payload\":{\"kind\":\"special\"}}")"
test "$(printf '%s' "$ev3" | jq -r .session)" = "$special_session"
test "$(PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- events pull --page demo --session "$special_session" | jq -r .payload.kind)" = special
special_ts="$(printf '%s' "$ev3" | jq -r .ts)"
test "$(PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- events pull --page demo --since "$special_ts" --until "$special_ts" | jq -r .payload.kind | tail -n 1)" = special
echo 'PASS: append-only event envelopes, snapshot version context, and filters'
