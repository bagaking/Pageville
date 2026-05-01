#!/usr/bin/env bash
set -euo pipefail

# Publishing must never dereference a symlink out of the published directory.
# A link such as `key.txt -> ~/.ssh/id_rsa` would otherwise land in the CAS
# permanently (v0 never GCs) and be served over HTTP.

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17785}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/site/nested"
printf 'SECRET-CANARY-DO-NOT-PUBLISH\n' > "$tmp_dir/secret.txt"
printf '<h1>symlink-page</h1>\n' > "$tmp_dir/site/index.html"
ln -s "$tmp_dir/secret.txt" "$tmp_dir/site/leak.txt"
ln -s "$tmp_dir/secret.txt" "$tmp_dir/site/nested/leak.txt"
ln -s "$tmp_dir" "$tmp_dir/site/loop-dir"

run() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- "$@"; }
run publish "$tmp_dir/site" --page symlink-demo >/dev/null

# The real file still publishes.
curl -fsS "http://127.0.0.1:${port}/symlink-demo/" | grep -q 'symlink-page'

# Neither symlink is reachable, at the root or nested.
test "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/symlink-demo/leak.txt")" = 404
test "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/symlink-demo/nested/leak.txt")" = 404

# The secret never entered the content-addressed store at all.
if grep -rqa 'SECRET-CANARY-DO-NOT-PUBLISH' "$tmp_dir/data" 2>/dev/null; then
  echo 'FAIL: symlink target leaked into the data dir' >&2
  exit 1
fi

echo 'PASS: publish skips symlinks; linked secrets never enter the CAS or the HTTP surface'
