#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17782}"
target="http://127.0.0.1:${port}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/site"
printf 'cli-surface\n' > "$tmp_dir/site/index.html"
run() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- "$@"; }
id="$(run publish "$tmp_dir/site" --page cli-demo | sed -n 's/.*snapshot_id=\([^ ]*\).*/\1/p')"
run pages list | grep -q '"page":"cli-demo"'
run versions list --page cli-demo | grep -q "$id"
run --json pages list | jq -e '.[0].page == "cli-demo"' >/dev/null
run --json versions list --page cli-demo | jq -e '.[0].snapshot_id != null' >/dev/null
curl -fsS -X POST "$target/api/v0/pages/cli-demo/events" -H 'content-type: application/json' -d "{\"version\":\"$id\",\"session\":\"cli-session\",\"payload\":{\"via\":\"cli\"}}" >/dev/null
run events pull --page cli-demo --session cli-session | jq -e '.payload.via == "cli"' >/dev/null
run --target "$target" events pull --page cli-demo --version "$id" | jq -e '.version == "'$id'"' >/dev/null
if run --target 'http://example.com:1234' pages list >/dev/null 2>&1; then exit 1; fi
echo 'PASS: pages/versions/events CLI, NDJSON/JSON output, and loopback --target'
