#!/usr/bin/env bash
# Guard against links to the former docs host.
#
# Why: the docs moved from docs.everruns.com to everruns.com/docs (2026-10-12,
# knowledge/ui/documentation.md). The old host 301s to the new address, so a
# stale link still works, but every one costs readers and agents a redirect
# hop, splits search ranking, and spreads the old address into published crate
# docs, skills and plugin manifests that outlive the redirect's owner.
#
# A tracked file may name docs.everruns.com only when it is about the move
# itself (redirect code and checks, changelog history). Those files are listed
# below; everything else links https://everruns.com/docs/.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

ALLOWED=(
  # Release and decision history: records what the address was.
  ':!CHANGELOG.md'
  ':!docs/changelog/2026-10-12-docs-move-to-everruns-com/'
  ':!knowledge/log.md'
  ':!knowledge/ui/documentation.md'
  # The redirect itself and the checks that prove it.
  ':!.github/workflows/docs-deploy.yml'
  ':!apps/docs/public/_redirects'
  ':!apps/docs/scripts/check-live.mjs'
  ':!apps/docs/scripts/stage-worker-assets.mjs'
  ':!apps/docs/site.config.mjs'
  ':!apps/docs/worker/'
  ':!apps/docs/wrangler.jsonc'
  # The chat UI still treats links to the old host as first-party.
  ':!apps/ui/src/components/markdown/markdown-link.tsx'
  ':!apps/ui/src/__tests__/streamdown-message.test.tsx'
  ':!scripts/lib/check-docs-host.sh'
)

if MATCHES="$(git grep -n -I 'docs\.everruns\.com' -- . "${ALLOWED[@]}")"; then
  echo "Links to the former docs host; use https://everruns.com/docs/ instead:"
  printf '%s\n' "$MATCHES" | sed 's/^/  /'
  exit 1
fi
echo "No links to docs.everruns.com outside the allowlist."
