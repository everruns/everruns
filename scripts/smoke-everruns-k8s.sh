#!/usr/bin/env bash
# Run `everruns k8s up` end to end with a CLI built from this checkout against
# the cluster in the current kube context (CI uses kind): first install with
# no prompts, CLI signed in through a port-forward, a second run that keeps
# the generated secrets, the hosted shape through --set, uninstall and a
# reinstall that asks nothing and keeps the data, and destroy.
#
# The chart comes from this checkout (`--chart charts/everruns`), the images
# are published ones (EVERRUNS_TAG, default `latest`). Settings and CLI
# credentials go to a temporary XDG home so the runner's profiles are never
# touched.
#
# Usage: scripts/smoke-everruns-k8s.sh [path-to-everruns-binary]
#   KEEP_RELEASE=1 leaves the install for inspection.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${1:-$ROOT/target/debug/everruns}"
TAG="${EVERRUNS_TAG:-latest}"
NAME="smoke-cli"
NS="everruns-cli"
PROFILE="k8s-$NAME"
PORT=19400

TMP="$(mktemp -d)"
export XDG_CONFIG_HOME="$TMP/config" XDG_DATA_HOME="$TMP/data"
export RUST_BACKTRACE=0
unset EVERRUNS_API_KEY EVERRUNS_API_URL EVERRUNS_PROFILE

step() { printf '\n==> %s\n' "$*"; }
fail() {
  echo "::error title=everruns k8s smoke::$*" >&2
  kubectl -n "$NS" get pods -o wide >&2 || true
  "$BIN" k8s logs --name "$NAME" --no-follow server >&2 || true
  exit 1
}
pf_pid=""
cleanup() {
  [ -n "$pf_pid" ] && kill "$pf_pid" 2>/dev/null || true
  if [ "${KEEP_RELEASE:-}" != "1" ]; then
    helm uninstall "$NAME" -n "$NS" >/dev/null 2>&1 || true
    kubectl delete namespace "$NS" --wait=false >/dev/null 2>&1 || true
    rm -rf "$TMP"
  fi
}
trap cleanup EXIT

up() {
  "$BIN" k8s up --name "$NAME" --namespace "$NS" --chart "$ROOT/charts/everruns" --image-tag "$TAG" \
    --port "$PORT" --yes --no-open "$@"
}
secrets() {
  { kubectl -n "$NS" get secret "$NAME-everruns-generated" -o jsonpath='{.data}'
    kubectl -n "$NS" get secret "$NAME-everruns-internal" -o jsonpath='{.data}'; } | sha256sum
}
# The profile points at the port-forward, which `up` closes when it is done.
with_forward() {
  "$BIN" k8s port-forward --name "$NAME" >/dev/null 2>&1 &
  pf_pid=$!
  for _ in $(seq 1 30); do curl -fsS "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break; sleep 1; done
  "$@"
  local rc=$?
  kill "$pf_pid" 2>/dev/null || true; wait "$pf_pid" 2>/dev/null || true; pf_pid=""
  return $rc
}
signed_in() {
  "$BIN" --profile "$PROFILE" whoami | grep -q "smoke@example.com" \
    && "$BIN" --profile "$PROFILE" agents list -o json >/dev/null
}

step "Tool and cluster checks refuse an install that is not set up"
"$BIN" k8s doctor --name "$NAME" || fail "doctor failed before install"

step "Without --yes and a terminal, up refuses to pick a cluster"
"$BIN" k8s up --name "$NAME" --namespace "$NS" --bundled-postgres </dev/null && fail "up installed without confirmation"

step "First install, no prompts"
up --bundled-postgres --admin-email smoke@example.com --provider skip || fail "first up failed"

step "CLI is signed in to the install"
with_forward signed_in || fail "the CLI profile does not work"

step "Second run keeps the secrets and the sign-in"
before="$(secrets)"
up || fail "second up failed"
[ "$before" = "$(secrets)" ] || fail "a second up changed the generated secrets"

step "The hosted shape through --set, kept on later runs"
up --set server.replicas=2 --set valkey.enabled=true || fail "up with two servers failed"
[ "$(kubectl -n "$NS" get deploy "$NAME-everruns-server" -o jsonpath='{.status.readyReplicas}')" = "2" ] \
  || fail "two servers are not ready"
up || fail "up after --set failed"
[ "$(kubectl -n "$NS" get deploy "$NAME-everruns-server" -o jsonpath='{.spec.replicas}')" = "2" ] \
  || fail "a later up dropped --set server.replicas=2"

step "Status"
"$BIN" k8s status --name "$NAME" 2>&1 | tee "$TMP/status.log"
grep -q "$NAME-everruns-server" "$TMP/status.log" || fail "status lists no server pod"

step "Doctor on the install"
# The system doctor may flag what a smoke install lacks (no provider key); the
# cluster checks must pass and the doctor must run in the server pod.
"$BIN" k8s doctor --name "$NAME" >"$TMP/doctor.log" 2>&1 || true
cat "$TMP/doctor.log"
grep -q "Permissions can install" "$TMP/doctor.log" || fail "doctor did not check the cluster"
grep -q "could not run in the server pod" "$TMP/doctor.log" && fail "the system doctor did not run in the server pod"

step "Uninstall keeps the data; up brings it back"
"$BIN" k8s uninstall --name "$NAME" || fail "uninstall failed"
kubectl -n "$NS" get secret "$NAME-everruns-generated" >/dev/null || fail "uninstall deleted the generated secrets"
up || fail "up after uninstall failed"
[ "$before" = "$(secrets)" ] || fail "reinstall changed the generated secrets"
with_forward signed_in || fail "the CLI profile does not work after reinstall"

step "Destroy deletes everything, the namespace too, and only after --yes"
"$BIN" k8s destroy --name "$NAME" </dev/null && fail "destroy went ahead without --yes"
"$BIN" k8s destroy --name "$NAME" --yes || fail "destroy failed"
[ -e "$XDG_DATA_HOME/everruns/k8s/$NAME" ] && fail "destroy left the local record"
kubectl -n "$NS" get secret "$NAME-everruns-generated" >/dev/null 2>&1 && fail "destroy left the generated secrets"
kubectl -n "$NS" get pvc -l "app.kubernetes.io/instance=$NAME" -o name | grep -q . && fail "destroy left volumes"
grep -q "\"$PROFILE\"" "$XDG_CONFIG_HOME/everruns/credentials.json" && fail "destroy left the CLI profile"
kubectl get namespace "$NS" >/dev/null 2>&1 && fail "destroy left the namespace"
"$BIN" k8s destroy --name "$NAME" --yes || fail "a second destroy failed"

step "everruns k8s smoke passed"
