#!/usr/bin/env bash
# Install the Helm chart on the cluster in the current kube context (CI uses a
# kind cluster), wait until it is healthy, sign in through the proxy, and check
# that an upgrade and a reinstall keep the generated secrets.
#
# Usage: scripts/smoke-helm-chart.sh   (needs helm, kubectl and curl)
#   EVERRUNS_TAG  image tag to run (default latest)
#   KEEP_RELEASE=1 leaves the release installed for inspection.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHART="$ROOT/charts/everruns"
NS="everruns-smoke"
REL="smoke"
TAG="${EVERRUNS_TAG:-latest}"
EMAIL="smoke@example.com"
SECRET="$REL-everruns-generated"

step() { printf '\n==> %s\n' "$*"; }
fail() {
  echo "::error title=helm chart smoke::$*" >&2
  kubectl -n "$NS" get pods -o wide >&2 || true
  kubectl -n "$NS" logs "deploy/$REL-everruns-server" --tail 80 >&2 || true
  exit 1
}
pf_pid=""
cleanup() {
  [ -n "$pf_pid" ] && kill "$pf_pid" 2>/dev/null || true
  if [ "${KEEP_RELEASE:-}" != "1" ]; then
    helm uninstall "$REL" -n "$NS" >/dev/null 2>&1 || true
    kubectl delete namespace "$NS" --wait=false >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

install() {
  helm upgrade --install "$REL" "$CHART" -n "$NS" --create-namespace \
    --set image.tag="$TAG" --set postgresql.enabled=true --set auth.adminEmail="$EMAIL" "$@"
}
secrets() { kubectl -n "$NS" get secret "$SECRET" -o jsonpath='{.data}' | sha256sum; }
wait_ready() {
  for deploy in server worker ui proxy; do
    kubectl -n "$NS" rollout status "deploy/$REL-everruns-$deploy" --timeout 600s \
      || fail "$deploy did not become ready"
  done
}

step "Install"
install || fail "helm install failed"
wait_ready

step "Reach it through the proxy and sign in"
kubectl -n "$NS" port-forward "svc/$REL-everruns" 19300:9300 >/dev/null 2>&1 &
pf_pid=$!
for _ in $(seq 1 30); do curl -fsS http://127.0.0.1:19300/health >/dev/null 2>&1 && break; sleep 1; done
curl -fsS http://127.0.0.1:19300/health || fail "/health through the proxy failed"
echo
curl -fsS -o /dev/null http://127.0.0.1:19300/ || fail "the UI did not answer through the proxy"
password="$(kubectl -n "$NS" get secret "$SECRET" -o jsonpath='{.data.AUTH_ADMIN_PASSWORD}' | base64 -d)"
curl -fsS -X POST http://127.0.0.1:19300/api/v1/auth/login -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$password\"}" | grep -q access_token || fail "admin sign-in failed"
kill "$pf_pid"; pf_pid=""

step "Upgrade keeps the generated secrets"
before="$(secrets)"
install --set worker.replicas=2 || fail "helm upgrade failed"
wait_ready
[ "$before" = "$(secrets)" ] || fail "helm upgrade changed the generated secrets"

step "Uninstall keeps them, and a reinstall uses them"
helm uninstall "$REL" -n "$NS" || fail "helm uninstall failed"
kubectl -n "$NS" get secret "$SECRET" >/dev/null || fail "uninstall deleted the generated secrets"
install || fail "reinstall failed"
[ "$before" = "$(secrets)" ] || fail "reinstall changed the generated secrets"

step "helm chart smoke passed"
