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

echo "==> Caddyfile matches infra/docker-compose/docker-compose.yaml"
python3 - "$ROOT/infra/docker-compose/docker-compose.yaml" "$CHART/files/Caddyfile" <<'PY'
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
# Everything the hosted service runs: several servers, network policies, the
# egress proxy, metrics, S3 and cloud identity.
everything=(--set postgresql.enabled=true --set auth.adminEmail=admin@example.com --set server.replicas=2
  --set valkey.enabled=true --set worker.autoscaling.enabled=true --set networkPolicy.enabled=true
  --set egressProxy.enabled=true --set metrics.enabled=true --set metrics.serviceMonitor.enabled=true
  --set storage.s3.enabled=true --set storage.s3.bucket=files --set serviceAccount.create=true
  --set ui.replicas=2 --set proxy.replicas=2)
render "${everything[@]}"

echo "==> rendered shapes say what they should"
out="$(helm template check "$CHART" "${everything[@]}")"
grep -q 'name: check-everruns-server-grpc' <<<"$out" || fail "no headless gRPC Service"
grep -q 'value: "check-everruns-server-grpc:9001"' <<<"$out" || fail "workers do not use the headless gRPC Service"
grep -q 'maxUnavailable: 0, maxSurge: 1' <<<"$out" || fail "several servers do not roll"
grep -q 'FROM session_actors' <<<"$out" || fail "worker autoscaling ignores chat turns"
grep -q 'kind: PodDisruptionBudget' <<<"$out" || fail "no PodDisruptionBudget"
grep -q '@check-everruns-valkey:6379' <<<"$out" || fail "Valkey URL missing"
grep -q 'everruns-egress-proxy"' <<<"$out" || fail "egress proxy not rendered"
grep -q 'name: METRICS_ADDR' <<<"$out" || fail "metrics address missing"
grep -q 'value: s3' <<<"$out" || fail "S3 storage missing"
one="$(helm template check "$CHART" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com)"
grep -q 'type: Recreate' <<<"$one" || fail "a single server must be replaced with Recreate"
if grep -q 'kind: NetworkPolicy' <<<"$one"; then fail "network policies render when off"; fi

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
refuses "needs Valkey" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com --set server.replicas=2
refuses "needs NATS" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com --set server.replicas=2 \
  --set valkey.enabled=true --set nats.enabled=false
refuses "needs storage.s3.bucket" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com --set storage.s3.enabled=true
refuses "cannot be used with auth.mode full" --set postgresql.enabled=true --set auth.mode=full \
  --set auth.adminEmail=admin@example.com --set seedDefaultProviderKeys=true

echo "==> a packaged chart runs its own release"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
helm package "$CHART" --version 1.2.3 --app-version 1.2.3 -d "$tmp" >/dev/null
helm template check "$tmp/everruns-1.2.3.tgz" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  | grep -q "ghcr.io/everruns/everruns-server:v1.2.3" || fail "packaged chart does not use v1.2.3 images"

echo "helm chart checks passed"
