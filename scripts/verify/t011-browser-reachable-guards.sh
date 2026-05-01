#!/usr/bin/env bash
set -euo pipefail

# The daemon binds 127.0.0.1, but that is not a trust boundary on its own: any
# page the user visits can reach it, and a rebound DNS name is treated as
# same-origin by the browser. These are the browser-reachable holes.

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
port="${PAGEVILLE_PORT:-17786}"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/site"
printf '<h1>guarded</h1>\n' > "$tmp_dir/site/index.html"
run() { PAGEVILLE_DATA_DIR="$tmp_dir/data" PAGEVILLE_PORT="$port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- "$@"; }
run publish "$tmp_dir/site" --page guarded >/dev/null

base="http://127.0.0.1:${port}"

# 1. DNS rebinding: a foreign Host must be refused on API and page routes.
test "$(curl -sS -o /dev/null -w '%{http_code}' -H 'Host: evil.example' "$base/api/v0/pages")" = 403
test "$(curl -sS -o /dev/null -w '%{http_code}' -H 'Host: evil.example:'"$port" "$base/api/v0/pages")" = 403
test "$(curl -sS -o /dev/null -w '%{http_code}' -H 'Host: evil.example' "$base/guarded/")" = 403
# Loopback spellings still work.
test "$(curl -sS -o /dev/null -w '%{http_code}' -H "Host: localhost:$port" "$base/api/v0/pages")" = 200
test "$(curl -sS -o /dev/null -w '%{http_code}' "$base/api/v0/pages")" = 200

# 2. Cross-site shutdown. A body-less POST is a CORS "simple request", so a
#    hostile page can fire it with no preflight to stop it.
test "$(curl -sS -o /dev/null -w '%{http_code}' -X POST -H 'Origin: https://evil.example' "$base/api/v0/shutdown")" = 403
# The daemon is still alive after the rejected attempt.
test "$(run daemon status)" = running

# 3. Served pages carry hardening headers and an explicit charset.
headers="$(curl -fsSI "$base/guarded/")"
printf '%s\n' "$headers" | grep -qi 'x-content-type-options: nosniff'
printf '%s\n' "$headers" | grep -qi 'content-security-policy:'
printf '%s\n' "$headers" | grep -qi 'content-type: text/html; charset=utf-8'

# 4. The local CLI (no Origin header) can still stop the daemon.
run daemon stop
for _ in $(seq 1 30); do test "$(run daemon status)" = stopped && break || sleep 0.1; done
test "$(run daemon status)" = stopped

echo 'PASS: Host pinning blocks DNS rebinding, cross-origin shutdown is refused, pages carry nosniff/CSP/charset'
