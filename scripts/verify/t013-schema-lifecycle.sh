#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp_dir="$(mktemp -d)"
legacy_port="${PAGEVILLE_PORT:-17784}"
bad_port="$((legacy_port + 1))"
trap 'PAGEVILLE_DATA_DIR="$tmp_dir/legacy" PAGEVILLE_PORT="$legacy_port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; PAGEVILLE_DATA_DIR="$tmp_dir/bad" PAGEVILLE_PORT="$bad_port" cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/legacy"
python3 - "$tmp_dir/legacy/pageville.db" <<'PY'
import sqlite3
import sys

conn = sqlite3.connect(sys.argv[1])
conn.executescript("""
CREATE TABLE pages(page TEXT PRIMARY KEY, latest TEXT NOT NULL);
CREATE TABLE snapshots(id TEXT PRIMARY KEY, page TEXT NOT NULL, created_at TEXT NOT NULL, spa INTEGER NOT NULL, manifest TEXT NOT NULL);
CREATE TABLE events(event_id TEXT PRIMARY KEY, page TEXT NOT NULL, version TEXT NOT NULL, session TEXT NOT NULL, ts TEXT NOT NULL, identity TEXT, payload TEXT NOT NULL);
""")
conn.commit()
conn.close()
PY

legacy_pages="$(PAGEVILLE_DATA_DIR="$tmp_dir/legacy" PAGEVILLE_PORT="$legacy_port" \
  cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- --json pages list)"
test "$legacy_pages" = '[]'
legacy_version="$(python3 - "$tmp_dir/legacy/pageville.db" <<'PY'
import sqlite3
import sys
conn = sqlite3.connect(sys.argv[1])
print(conn.execute('PRAGMA user_version').fetchone()[0])
conn.close()
PY
)"
test "$legacy_version" = 1
PAGEVILLE_DATA_DIR="$tmp_dir/legacy" PAGEVILLE_PORT="$legacy_port" \
  cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon stop

mkdir -p "$tmp_dir/bad"
python3 - "$tmp_dir/bad/pageville.db" <<'PY'
import sqlite3
import sys

conn = sqlite3.connect(sys.argv[1])
conn.execute('CREATE TABLE pages(page TEXT PRIMARY KEY)')
conn.commit()
conn.close()
PY

set +e
bad_output="$(PAGEVILLE_DATA_DIR="$tmp_dir/bad" PAGEVILLE_PORT="$bad_port" \
  cargo run --quiet --manifest-path "$root_dir/Cargo.toml" -- daemon run 2>&1)"
bad_status=$?
set -e
test "$bad_status" -ne 0
printf '%s\n' "$bad_output" | grep -qi 'schema'
if curl -fsS -m 1 "http://127.0.0.1:${bad_port}/api/v0/health" >/dev/null 2>&1; then
  echo 'FAIL: daemon served traffic with an incompatible schema' >&2
  exit 1
fi

echo 'PASS: legacy schema adoption and incompatible-schema startup failure'
