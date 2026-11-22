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
m = re.search(r"\n  caddyfile:\n    content: \|\n(.*?)\n  (?=[#a-z_])", compose, re.S)
if not m:
    sys.exit("caddyfile config not found in the compose file")
# The next config's comment or key ends it; a Caddyfile ends with its site block.
if not m.group(1).rstrip().endswith("}"):
    sys.exit("the compose caddyfile config does not end with its site block")
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

echo "==> data plane v2 by default, v1 one setting away"
grep -q 'name: check-agentd$' <<<"$one" || fail "v2 is the default but no shard renders"
grep -q 'EVERRUNS_DEFAULT_DATA_PLANE_URL, value: "http://check-agentd-gateway:7000"' <<<"$one" \
  || fail "the server does not reach the shard through the gateway Service over its ready nodes"
grep -q 'INJECTED_CAPABILITIES_BASE_URL, value: "http://check-everruns-server:9000/api"' <<<"$one" \
  || fail "the server hands the data plane injected capabilities at an address it cannot reach"
[ "$(grep -c 'AGENTD_CONTROL_PLANE, value: "http://check-everruns-server:9000/api"' <<<"$one")" -ge 3 ] \
  || fail "a data plane pod (node, executor, edge) does not know its control plane"
grep -q 'reverse_proxy check-everruns-edge:7100 check-everruns-server:9000' <<<"$one" \
  || fail "the proxy does not send session calls to the edge router"
grep -q 'AGENTD_STORE, value: "file:///store"' <<<"$one" || fail "without S3 the shards do not share a store volume"
grep -q 'AGENTD_STORE, value: "s3://files/shards"' <<<"$out" || fail "with S3 the shards do not keep history there"
v1="$(helm template check "$CHART" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set dataPlane.version=v1)"
if grep -q 'agentd' <<<"$v1"; then fail "v1 renders the data plane"; fi
if grep -q 'check-everruns-edge' <<<"$v1"; then fail "v1 routes to an edge router that does not run"; fi
grep -q 'EVERRUNS_DATA_PLANE, value: "v1"' <<<"$v1" || fail "v1 does not tell the server"
kept="$(helm template check "$CHART" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set dataPlane.version=v1 --set dataPlane.keepRunning=true)"
grep -q 'name: check-agentd$' <<<"$kept" || fail "keepRunning does not keep the shard"
if grep -q 'EVERRUNS_DEFAULT_DATA_PLANE_URL' <<<"$kept"; then fail "v1 still places new organizations on v2"; fi
three="$(helm template check "$CHART" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set dataPlane.shard.nodes=3)"
grep -q 'check-agentd-2.check-agentd:7000' <<<"$three" || fail "three nodes are not in the topology"

echo "==> Ingress defaults to direct routing"
direct=(--set postgresql.enabled=true --set auth.adminEmail=admin@example.com
  --set ingress.enabled=true --set ingress.host=everruns.example.com --set networkPolicy.enabled=true
  --set 'networkPolicy.ingressFrom[0].namespaceSelector.matchLabels.kubernetes\.io/metadata\.name=ingress-system')
for mode in v1 v2 keepRunning; do
  data_plane=(--set "dataPlane.version=$mode")
  if [[ "$mode" == keepRunning ]]; then
    data_plane=(--set dataPlane.version=v1 --set dataPlane.keepRunning=true)
  fi
  helm template check "$CHART" "${direct[@]}" "${data_plane[@]}" | python3 -c '
import re, sys
mode = sys.argv[1]
docs = sys.stdin.read().split("\n---")
assert not any("component: proxy\n" in doc for doc in docs), "Caddy still renders"
assert not any("name: check-everruns-proxy\n" in doc for doc in docs), "Caddy ConfigMap still renders"
ingress, = (doc for doc in docs if "\nkind: Ingress\n" in doc)
assert "host: \"everruns.example.com\"" in ingress
routes = dict((path, (path_type, service)) for path, path_type, service in re.findall(
    r"path: (\S+)\s+pathType: (\S+)\s+backend:\s+service:\s+name: check-everruns-(\w+)\s+port: \{ name: http \}",
    ingress,
))
expected = {"/": ("Prefix", "ui")}
expected.update({path: ("Prefix", "server") for path in ("/api", "/oauth", "/.well-known", "/api-doc")})
expected.update({path: ("Exact", "server") for path in ("/mcp", "/health", "/llms.txt", "/auth.md", "/robots.txt", "/sitemap.xml")})
if mode != "v1":
    expected.update({"/api/v1/" + route: ("Prefix", "edge") for route in ("sessions", "commands", "workspaces")})
assert routes == expected, routes
for service in {backend for _, backend in expected.values()}:
    name = f"check-everruns-{service}"
    assert any("\nkind: Service\n" in doc and f"name: {name}\n" in doc for doc in docs), name
    policy, = (doc for doc in docs if "\nkind: NetworkPolicy\n" in doc and f"name: {name}\n" in doc)
    assert "kubernetes.io/metadata.name: ingress-system" in policy, name
ui, = (doc for doc in docs if "\nkind: Deployment\n" in doc and "component: ui\n" in doc)
assert "UI_HOMEPAGE_MARKDOWN_URL" in ui
' "$mode"
done
if grep -q 'UI_HOMEPAGE_MARKDOWN_URL' <<<"$one"; then fail "the UI takes over discovery with Caddy enabled"; fi
legacy="$(helm template check "$CHART" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set ingress.enabled=true --set proxy.enabled=true)"
grep -q 'component: proxy' <<<"$legacy" || fail "proxy.enabled=true must keep Caddy with Ingress"

echo "==> isolated sandboxes render least privilege"
sbx="$(helm template check "$CHART" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set egressProxy.enabled=true --set networkPolicy.enabled=true --set sandboxes.kubernetes.enabled=true \
  --set sandboxes.kubernetes.runtimeClassName=isolated-sandbox)"
python3 - "$sbx" <<'PY'
import sys
docs = [d for d in sys.argv[1].split("\n---") if d.strip()]
def find(kind, name=None):
    return [d for d in docs if f"\nkind: {kind}\n" in "\n" + d and (name is None or f"name: {name}\n" in d)]
role = find("Role")
assert len(role) == 1, "one Role"
assert "namespace: everruns-sandboxes" in role[0], "Role is namespaced to the sandboxes"
assert "ClusterRole" not in sys.argv[1], "no cluster-wide rights"
assert "pods/exec" not in role[0], "the guest, not kubectl exec, runs commands"
deny = find("NetworkPolicy", "default-deny")
assert deny and "policyTypes: [Ingress, Egress]" in deny[0], "sandbox namespace starts closed"
assert "pod-security.kubernetes.io/enforce: baseline" in sys.argv[1], "no privileged sandbox pods"
for component in ("server", "worker"):
    dep = [d for d in find("Deployment") if f"app.kubernetes.io/component: {component}\n" in d][0]
    assert "automountServiceAccountToken: true" in dep, f"{component} cannot reach the API"
    assert "KUBERNETES_SANDBOX_RUNTIME_CLASS, value: \"isolated-sandbox\"" in dep, component
    assert "KUBERNETES_SANDBOX_EGRESS_PROXY_ADDRESS" in dep, component
proxy = [d for d in find("Deployment") if "component: egress-proxy" in d][0]
assert "EGRESS_PROXY_SOCKS_ADDR" in proxy, "the proxy opens its SOCKS listener"
PY
if helm template check "$CHART" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  | grep -q 'automountServiceAccountToken: true'; then
  fail "the API token is mounted without isolated sandboxes"
fi

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
refuses "needs egressProxy.enabled" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set sandboxes.kubernetes.enabled=true
refuses "must be proxy, direct or none" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set sandboxes.kubernetes.enabled=true --set sandboxes.kubernetes.egress=open
refuses "must differ from the release namespace" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set sandboxes.kubernetes.enabled=true --set sandboxes.kubernetes.egress=none --set sandboxes.kubernetes.namespace=default
refuses "needs storage.s3.bucket" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com --set storage.s3.enabled=true
refuses "must be v1 or v2" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set dataPlane.version=v3
refuses "must be 1, 3 or 5" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set dataPlane.shard.nodes=2
refuses "needs ingress.enabled=true" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set proxy.enabled=false
refuses "needs networkPolicy.ingressFrom" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  --set ingress.enabled=true --set networkPolicy.enabled=true
refuses "cannot be used with auth.mode full" --set postgresql.enabled=true --set auth.mode=full \
  --set auth.adminEmail=admin@example.com --set seedDefaultProviderKeys=true

echo "==> a packaged chart runs its own release"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
helm package "$CHART" --version 1.2.3 --app-version 1.2.3 -d "$tmp" >/dev/null
helm template check "$tmp/everruns-1.2.3.tgz" --set postgresql.enabled=true --set auth.adminEmail=admin@example.com \
  | grep -q "ghcr.io/everruns/everruns-server:v1.2.3" || fail "packaged chart does not use v1.2.3 images"

echo "helm chart checks passed"
