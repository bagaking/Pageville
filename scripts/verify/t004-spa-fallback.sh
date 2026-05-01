#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17780}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/spa" "$tmp_dir/plain"
printf '<h1>spa-index</h1>\n' > "$tmp_dir/spa/index.html"
printf 'body{}\n' > "$tmp_dir/spa/app.css"
printf 'console.log(1);\n' > "$tmp_dir/spa/app.js"
printf 'binary\n' > "$tmp_dir/spa/data.bin"
printf '<h1>plain-index</h1>\n' > "$tmp_dir/plain/index.html"

publish() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- publish "$1" --page "$2" "${@:3}"; }
publish "$tmp_dir/spa" spa --spa >/dev/null
publish "$tmp_dir/plain" plain >/dev/null
test "$(curl -fsS "http://127.0.0.1:${port}/spa/deep/link")" = '<h1>spa-index</h1>'
test "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/plain/deep/link")" = 404
curl -fsSI "http://127.0.0.1:${port}/spa/deep/link" | grep -qi 'content-type: text/html'
curl -fsSI "http://127.0.0.1:${port}/spa/app.css" | grep -qi 'content-type: text/css'
curl -fsSI "http://127.0.0.1:${port}/spa/app.js" | grep -qi 'content-type: text/javascript\|content-type: application/javascript'
curl -fsSI "http://127.0.0.1:${port}/spa/data.bin" | grep -qi 'content-type: application/octet-stream'
echo 'PASS: SPA fallback, non-SPA 404, and extension MIME semantics'
