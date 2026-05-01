#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17777}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/site"
printf '<h1>pageville-t001</h1>\n' > "$tmp_dir/site/index.html"

output="$(PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$tmp_dir/site" --page demo)"
printf '%s\n' "$output" | grep -q 'snapshot_id='
curl -fsS "http://127.0.0.1:${port}/demo/" | grep -q 'pageville-t001'
echo 'PASS: publish -> auto-start daemon -> /demo/ serves index.html'
