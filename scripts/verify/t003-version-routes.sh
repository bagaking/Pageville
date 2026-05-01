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
for _ in $(seq 1 20); do curl -fsS "http://127.0.0.1:${port}/demo/" | grep -Eq 'version-two|version-one'; done
echo 'PASS: pinned versions, latest aliases, slug validation, and stable latest reads'
