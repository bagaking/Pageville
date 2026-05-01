#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
echo 'Pageville v0 acceptance (clean per-slice environments)'
for check in t001-publish-serve.sh t002-cas-idempotent.sh t003-version-routes.sh t004-spa-fallback.sh t005-events-envelope.sh t006-cli-surface.sh t007-daemon-lifecycle.sh; do
  bash "$root_dir/scripts/verify/$check"
done
echo 'PASS: PRD §9 acceptance standards 1-7'
