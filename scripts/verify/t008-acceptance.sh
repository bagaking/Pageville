#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Fail fast on a missing tool rather than midway through a slice with a
# confusing error.
for tool in curl jq python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing required tool: $tool" >&2; exit 1; }
done

echo 'Pageville v0 acceptance (clean per-slice environments)'
for check in \
  t001-publish-serve.sh \
  t002-cas-idempotent.sh \
  t003-version-routes.sh \
  t004-spa-fallback.sh \
  t005-events-envelope.sh \
  t006-cli-surface.sh \
  t007-daemon-lifecycle.sh \
  t009-atlas-root.sh \
  t010-publish-symlink-escape.sh \
  t011-browser-reachable-guards.sh \
  t012-routing-and-time-filters.sh; do
  bash "$root_dir/scripts/verify/$check" || { echo "FAIL: $check exited $?" >&2; exit 1; }
done
echo 'PASS: PRD §9 acceptance standards 1-7 plus security and data-plane regressions'
