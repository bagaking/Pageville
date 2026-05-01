#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17784}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/site"
printf '<h1>atlas-a</h1>\n' > "$tmp_dir/site/index.html"
run() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- "$@"; }
run publish "$tmp_dir/site" --page atlas-a >/dev/null
printf '<h1>atlas-b</h1>\n' > "$tmp_dir/site/index.html"
run publish "$tmp_dir/site" --page atlas-b --spa >/dev/null

headers="$(curl -fsSI "http://127.0.0.1:${port}/")"
printf '%s\n' "$headers" | grep -qi 'content-type: text/html'
printf '%s\n' "$headers" | grep -qi 'cache-control: no-store'
root_html="$(curl -fsS "http://127.0.0.1:${port}/")"
printf '%s' "$root_html" | grep -q 'Pageville / Atlas'
printf '%s' "$root_html" | grep -q '/api/v0/pages'
printf '%s' "$root_html" | grep -q 'Publish handoff'
test "$(curl -fsS "http://127.0.0.1:${port}/api/v0/pages" | jq 'length')" -eq 2
echo 'PASS: embedded Atlas root exposes real page index and safe response headers'
