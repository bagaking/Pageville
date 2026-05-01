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

# Traversal is rejected per path COMPONENT, not by substring: a legal filename
# that merely contains `..` must publish and serve, while real traversal stays
# blocked. The substring check failed the ENTIRE snapshot on `data..old.json`.
mkdir -p "$tmp_dir/dots/sub"
printf 'ROOT\n' > "$tmp_dir/dots/index.html"
printf 'OLD\n' > "$tmp_dir/dots/data..old.json"
printf 'LEAD\n' > "$tmp_dir/dots/..lead"
printf 'NESTED\n' > "$tmp_dir/dots/sub/a..b.txt"
run publish "$tmp_dir/dots" --page dots >/dev/null
test "$(curl -fsS "http://127.0.0.1:${port}/dots/data..old.json")" = 'OLD'
test "$(curl -fsS "http://127.0.0.1:${port}/dots/..lead")" = 'LEAD'
test "$(curl -fsS "http://127.0.0.1:${port}/dots/sub/a..b.txt")" = 'NESTED'
# Real traversal, on both the publish and the serve side, is still refused.
for bad in '../etc/passwd' 'a/../../b' '/abs' 'a//b' './x'; do
  code="$(curl -sS -o /dev/null -w '%{http_code}' -X POST "http://127.0.0.1:${port}/api/v0/pages/dots/snapshots" \
    -H 'content-type: application/json' -d "{\"files\":{\"$bad\":\"eA==\"},\"spa\":false}")"
  test "$code" = 400 || { echo "FAIL: publish accepted traversal path '$bad' ($code)" >&2; exit 1; }
done
test "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/dots/%2e%2e%2f%2e%2e%2fetc%2fpasswd")" = 400

echo 'PASS: publish skips symlinks; linked secrets never enter the CAS or the HTTP surface'
