#!/usr/bin/env bash
# Static checks for the Helm chart: lint, render the supported shapes, refuse
# the unsupported ones with a clear message, and keep the chart's Caddyfile
# identical to the Docker Compose example's, so both route the same way.
#
# Usage: scripts/check-helm-chart.sh   (needs helm and python3)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHART="$ROOT/charts/everruns"
fail() { echo "::error title=helm chart::$*" >&2; exit 1; }

echo "==> Caddyfile matches examples/docker-compose-full.yaml"
python3 - "$ROOT/examples/docker-compose-full.yaml" "$CHART/files/Caddyfile" <<'PY'
import re, sys
compose, chart = (open(p).read() for p in sys.argv[1:])
m = re.search(r"\n  caddyfile:\n    content: \|\n(.*?)\n  prometheus_scrape_config:", compose, re.S)
if not m:
    sys.exit("caddyfile config not found in the compose file")
lines = [l[6:] if l.startswith("      ") else l.strip() for l in m.group(1).split("\n")]
if "\n".join(lines).rstrip() + "\n" != chart:
    sys.exit("charts/everruns/files/Caddyfile differs from the compose Caddyfile; copy it over")
PY

echo "==> helm lint"
helm lint "$CHART" --strict --set postgresql.enabled=true --set auth.adminEmail=admin@example.com

render() { helm template check "$CHART" "$@" >/dev/null; }

echo "==> renders the supported shapes"
render --set postgresql.enabled=true --set auth.adminEmail=admin@example.com
render --set database.url=postgres://u:p@db:5432/everruns --set auth.mode=full --set auth.adminEmail=admin@example.com \
  --set worker.autoscaling.enabled=true --set valkey.enabled=true --set nats.enabled=false
render --set database.existingSecret=db --set secrets.existingSecret=everruns --set auth.mode=none --set deploymentGrade=dev \
  --set ingress.enabled=true --set ingress.host=everruns.example.com --set providerKeys.existingSecret=keys

echo "==> refuses unsupported shapes"
refuses() {
  local want="$1"; shift
  local out
  if out="$(helm template check "$CHART" "$@" 2>&1)"; then
    fail "rendered without error: $*"
  fi
  grep -q "$want" <<<"$out" || fail "expected '$want' for: $* (got: $out)"
}
refuses "set database.url" --set auth.adminEmail=admin@example.com
refuses "set auth.adminEmail" --set postgresql.enabled=true
refuses "needs deploymentGrade dev" --set postgresql.enabled=true --set auth.mode=none
refuses "cannot be used with auth.mode full" --set postgresql.enabled=true --set auth.mode=full \
  --set auth.adminEmail=admin@example.com --set seedDefaultProviderKeys=true

echo "==> a packaged chart runs its own release"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
helm package "$CHART" --version 1.2.3 --app-version 1.2.3 -d "$tmp" >/dev/null
helm template check "$tmp/everruns-1.2.3.tgz" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  | grep -q "ghcr.io/everruns/everruns-server:v1.2.3" || fail "packaged chart does not use v1.2.3 images"

echo "helm chart checks passed"
