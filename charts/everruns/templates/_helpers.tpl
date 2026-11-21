{{- define "everruns.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 50 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 50 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "everruns.labels" -}}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{/* Selector labels for one component: pass (list $ "server"). */}}
{{- define "everruns.selector" -}}
{{- $ := index . 0 -}}
app.kubernetes.io/name: {{ $.Chart.Name }}
app.kubernetes.io/instance: {{ $.Release.Name }}
app.kubernetes.io/component: {{ index . 1 }}
{{- end -}}

{{- define "everruns.name" -}}
{{- $ := index . 0 -}}
{{- printf "%s-%s" (include "everruns.fullname" $) (index . 1) -}}
{{- end -}}

{{- define "everruns.image" -}}
{{- $ := index . 0 -}}
{{- $release := ternary "latest" (printf "v%s" $.Chart.AppVersion) (eq $.Chart.AppVersion "latest") -}}
{{- printf "%s/everruns-%s:%s" $.Values.image.registry (index . 1) ($.Values.image.tag | default $release) -}}
{{- end -}}

{{/* Secret with the generated values, or the one the person manages. */}}
{{- define "everruns.secretName" -}}
{{- .Values.secrets.existingSecret | default (printf "%s-generated" (include "everruns.fullname" .)) -}}
{{- end -}}

{{- define "everruns.configSecretName" -}}
{{- printf "%s-config" (include "everruns.fullname" .) -}}
{{- end -}}

{{/* Where DATABASE_URL comes from: (dict "name" ... "key" ...). */}}
{{- define "everruns.databaseSecret" -}}
{{- if .Values.postgresql.enabled -}}
name: {{ include "everruns.secretName" . }}
key: DATABASE_URL
{{- else if .Values.database.existingSecret -}}
name: {{ .Values.database.existingSecret }}
key: {{ .Values.database.existingSecretKey }}
{{- else -}}
name: {{ include "everruns.configSecretName" . }}
key: DATABASE_URL
{{- end -}}
{{- end -}}

{{/* Generated passwords for the bundled datastores and the egress proxy. */}}
{{- define "everruns.internalSecretName" -}}
{{- printf "%s-internal" (include "everruns.fullname" .) -}}
{{- end -}}

{{- define "everruns.hasNats" -}}
{{- if or .Values.nats.url .Values.nats.enabled }}true{{ end -}}
{{- end -}}

{{- define "everruns.hasValkey" -}}
{{- if or .Values.valkey.url .Values.valkey.enabled }}true{{ end -}}
{{- end -}}

{{/*
Env entries for NATS_URL. The bundled NATS requires a password; Kubernetes
expands $(NATS_PASSWORD) from the entry before it, so the password never
appears in the manifest.
*/}}
{{- define "everruns.natsEnv" -}}
{{- if .Values.nats.url }}
- { name: NATS_URL, value: {{ .Values.nats.url | quote }} }
{{- else if .Values.nats.enabled }}
- name: NATS_PASSWORD
  valueFrom: { secretKeyRef: { name: {{ include "everruns.internalSecretName" . }}, key: NATS_PASSWORD } }
- { name: NATS_URL, value: {{ printf "nats://everruns:$(NATS_PASSWORD)@%s:4222" (include "everruns.name" (list . "nats")) | quote }} }
{{- end }}
{{- end -}}

{{/* Env entries for VALKEY_URL, the same way. */}}
{{- define "everruns.valkeyEnv" -}}
{{- if .Values.valkey.url }}
- { name: VALKEY_URL, value: {{ .Values.valkey.url | quote }} }
{{- else if .Values.valkey.enabled }}
- name: VALKEY_PASSWORD
  valueFrom: { secretKeyRef: { name: {{ include "everruns.internalSecretName" . }}, key: VALKEY_PASSWORD } }
- { name: VALKEY_URL, value: {{ printf "redis://:$(VALKEY_PASSWORD)@%s:6379" (include "everruns.name" (list . "valkey")) | quote }} }
{{- end }}
{{- end -}}

{{/* Env entries that send tenant traffic through the egress proxy. */}}
{{- define "everruns.egressProxyEnv" -}}
{{- if .Values.egressProxy.enabled }}
- { name: EVERRUNS_EGRESS_PROXY_URL, value: {{ printf "http://%s:4750" (include "everruns.name" (list . "egress-proxy")) | quote }} }
- name: EVERRUNS_EGRESS_PROXY_SECRET
  valueFrom: { secretKeyRef: { name: {{ include "everruns.internalSecretName" . }}, key: EVERRUNS_EGRESS_PROXY_SECRET } }
{{- end }}
{{- end -}}

{{/* Env entries for isolated sandboxes, on server and workers. */}}
{{- define "everruns.sandboxEnv" -}}
{{- with .Values.sandboxes.kubernetes }}
{{- if .enabled }}
- { name: KUBERNETES_SANDBOX_NAMESPACE, value: {{ .namespace | quote }} }
{{- with .runtimeClassName }}
- { name: KUBERNETES_SANDBOX_RUNTIME_CLASS, value: {{ . | quote }} }
{{- end }}
- { name: KUBERNETES_SANDBOX_IMAGE, value: {{ .image | default (printf "ghcr.io/everruns/sandbox-dev:%s" (ternary "latest" (printf "v%s" $.Chart.AppVersion) (eq $.Chart.AppVersion "latest"))) | quote }} }
{{- with .desktopImage }}
- { name: KUBERNETES_SANDBOX_DESKTOP_IMAGE, value: {{ . | quote }} }
{{- end }}
{{- with .allowedImages }}
- { name: KUBERNETES_SANDBOX_ALLOWED_IMAGES, value: {{ . | quote }} }
{{- end }}
- { name: KUBERNETES_SANDBOX_SIZES, value: {{ join "," .sizes | quote }} }
{{- with .storageClassName }}
- { name: KUBERNETES_SANDBOX_STORAGE_CLASS, value: {{ . | quote }} }
{{- end }}
- { name: KUBERNETES_SANDBOX_WORKSPACE_SIZE, value: {{ .workspaceSize | quote }} }
{{- with .nodeSelector }}
- { name: KUBERNETES_SANDBOX_NODE_SELECTOR, value: {{ toJson . | quote }} }
{{- end }}
{{- with .tolerations }}
- { name: KUBERNETES_SANDBOX_TOLERATIONS, value: {{ toJson . | quote }} }
{{- end }}
- { name: KUBERNETES_SANDBOX_MAX_PER_ORG, value: {{ .maxPerOrg | quote }} }
{{- with .billingNote }}
- { name: KUBERNETES_SANDBOX_BILLING_NOTE, value: {{ . | quote }} }
{{- end }}
- { name: KUBERNETES_SANDBOX_GUEST_ROUTE, value: pod_ip }
- { name: KUBERNETES_SANDBOX_CONTROL_POD_LABELS, value: {{ printf "app.kubernetes.io/name=%s,app.kubernetes.io/instance=%s" $.Chart.Name $.Release.Name | quote }} }
- { name: KUBERNETES_SANDBOX_CONTROL_NAMESPACE, value: {{ $.Release.Namespace | quote }} }
- { name: KUBERNETES_SANDBOX_EGRESS, value: {{ .egress | quote }} }
{{- if eq .egress "proxy" }}
{{- $proxy := include "everruns.name" (list $ "egress-proxy") }}
- { name: KUBERNETES_SANDBOX_EGRESS_PROXY_ADDRESS, value: {{ printf "%s.%s.svc:1080" $proxy $.Release.Namespace | quote }} }
- { name: KUBERNETES_SANDBOX_EGRESS_PROXY_POD_LABELS, value: {{ printf "app.kubernetes.io/name=%s,app.kubernetes.io/instance=%s,app.kubernetes.io/component=egress-proxy" $.Chart.Name $.Release.Name | quote }} }
- { name: KUBERNETES_SANDBOX_EGRESS_PROXY_NAMESPACE, value: {{ $.Release.Namespace | quote }} }
# Sandboxes reach the proxy's plain in-cluster listener above with
# credentials signed by the proxy secret. Set a public TLS
# EVERRUNS_EGRESS_PROXY_SOCKS_ADDRESS in extraEnv to route E2B sandboxes too.
{{- end }}
{{- end }}
{{- end }}
{{- end -}}

{{/* The ServiceAccount server and workers run as when they manage sandboxes. */}}
{{- define "everruns.sandboxServiceAccount" -}}
{{- include "everruns.serverServiceAccount" . | default "default" -}}
{{- end -}}

{{- define "everruns.serverServiceAccount" -}}
{{- if .Values.serviceAccount.create -}}
{{- .Values.serviceAccount.name | default (include "everruns.name" (list . "server")) -}}
{{- else -}}
{{- .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/* A NetworkPolicy peer for one component: pass (list $ "server"). */}}
{{- define "everruns.peer" -}}
- podSelector:
    matchLabels: {{- include "everruns.selector" . | nindent 6 }}
{{- end -}}

{{/* Omitted or null means the presence of an Ingress chooses the routing mode. */}}
{{- define "everruns.proxyEnabled" -}}
{{- if kindIs "bool" .Values.proxy.enabled -}}
{{- if .Values.proxy.enabled }}true{{ end -}}
{{- else if not .Values.ingress.enabled }}true{{ end -}}
{{- end -}}

{{/* Stop with a clear message before anything is created. */}}
{{- define "everruns.validate" -}}
{{- if not (include "everruns.proxyEnabled" .) }}
{{- if not .Values.ingress.enabled }}
{{- fail "proxy.enabled=false needs ingress.enabled=true" }}
{{- end }}
{{- if and .Values.networkPolicy.enabled (not .Values.networkPolicy.ingressFrom) }}
{{- fail "direct Ingress with networkPolicy.enabled=true needs networkPolicy.ingressFrom, or set proxy.enabled=true to use Caddy" }}
{{- end }}
{{- end }}
{{- with .Values.sandboxes.kubernetes }}
{{- if .enabled }}
{{- if not (has .egress (list "proxy" "direct" "none")) }}
{{- fail (printf "sandboxes.kubernetes.egress must be proxy, direct or none, got %q" .egress) -}}
{{- end }}
{{- if and (eq .egress "proxy") (not $.Values.egressProxy.enabled) }}
{{- fail "sandboxes.kubernetes.egress=proxy needs egressProxy.enabled=true (or set egress to direct or none)" -}}
{{- end }}
{{- if eq .namespace $.Release.Namespace }}
{{- fail "sandboxes.kubernetes.namespace must differ from the release namespace" -}}
{{- end }}
{{- end }}
{{- end }}
{{- $modes := list "admin" "full" "none" -}}
{{- if not (has .Values.auth.mode $modes) -}}
{{- fail (printf "auth.mode must be admin, full or none, got %q" .Values.auth.mode) -}}
{{- end -}}
{{- if and (eq .Values.auth.mode "none") (ne .Values.deploymentGrade "dev") -}}
{{- fail "auth.mode none needs deploymentGrade dev: the server refuses no sign-in on a prod deployment" -}}
{{- end -}}
{{- if and (ne .Values.auth.mode "none") (not .Values.auth.adminEmail) -}}
{{- fail "set auth.adminEmail: the admin account signs in with it" -}}
{{- end -}}
{{- if and (eq .Values.auth.mode "full") .Values.seedDefaultProviderKeys -}}
{{- fail "seedDefaultProviderKeys cannot be used with auth.mode full: anyone who signs up would share the keys" -}}
{{- end -}}
{{- if not (or .Values.postgresql.enabled .Values.database.url .Values.database.existingSecret) -}}
{{- fail "set database.url or database.existingSecret, or postgresql.enabled=true to try the chart with a bundled database" -}}
{{- end -}}
{{- if gt (int .Values.server.replicas) 1 -}}
{{- if not (include "everruns.hasNats" .) -}}
{{- fail "server.replicas above 1 needs NATS (nats.enabled or nats.url): without it a live stream on one server misses events another server writes" -}}
{{- end -}}
{{- if not (include "everruns.hasValkey" .) -}}
{{- fail "server.replicas above 1 needs Valkey (valkey.enabled or valkey.url): without it each server keeps its own rate limits, multiplying them" -}}
{{- end -}}
{{- end -}}
{{- if and .Values.storage.s3.enabled (not .Values.storage.s3.bucket) -}}
{{- fail "storage.s3.enabled needs storage.s3.bucket" -}}
{{- end -}}
{{- if not (has .Values.dataPlane.version (list "v1" "v2")) -}}
{{- fail (printf "dataPlane.version must be v1 or v2, got %q" .Values.dataPlane.version) -}}
{{- end -}}
{{- if not (has (int .Values.dataPlane.cell.nodes) (list 1 3 5)) -}}
{{- fail "dataPlane.cell.nodes must be 1, 3 or 5: a cell needs a majority of its nodes" -}}
{{- end -}}
{{- if and .Values.metrics.serviceMonitor.enabled (not .Values.metrics.enabled) -}}
{{- fail "metrics.serviceMonitor.enabled needs metrics.enabled" -}}
{{- end -}}
{{- end -}}

{{/* Whether the data plane v2 components run (see values: dataPlane). */}}
{{- define "everruns.dataPlaneRuns" -}}
{{- if or (eq .Values.dataPlane.version "v2") .Values.dataPlane.keepRunning }}true{{ end -}}
{{- end -}}

{{- define "everruns.cellImage" -}}
{{- .Values.dataPlane.image | default (include "everruns.image" (list . "cell")) -}}
{{- end -}}

{{/* A cell node's gateway address: pass (list $ ordinal). */}}
{{- define "everruns.cellAddr" -}}
{{- $ := index . 0 -}}
{{- $cell := include "everruns.name" (list $ "cell") -}}
{{- printf "%s-%d.%s:7000" $cell (int (index . 1)) $cell -}}
{{- end -}}

{{/* Where cells keep session history: dataPlane.store, S3, or the shared volume. */}}
{{- define "everruns.cellStore" -}}
{{- if .Values.dataPlane.store -}}
{{- .Values.dataPlane.store -}}
{{- else if .Values.storage.s3.enabled -}}
{{- $prefix := trimSuffix "/" .Values.storage.s3.prefix -}}
{{- if $prefix -}}
{{- printf "s3://%s/%s/cells" .Values.storage.s3.bucket $prefix -}}
{{- else -}}
{{- printf "s3://%s/cells" .Values.storage.s3.bucket -}}
{{- end -}}
{{- else -}}
file:///store
{{- end -}}
{{- end -}}

{{/* Whether cells and executors mount the shared store volume. */}}
{{- define "everruns.cellStoreVolume" -}}
{{- if hasPrefix "file://" (include "everruns.cellStore" .) }}true{{ end -}}
{{- end -}}

{{/*
Env entries every data plane pod shares: the topology, the store and its
S3 settings (from storage.s3 when the store is there), the data plane it
is and the link to the server.
*/}}
{{- define "everruns.cellEnv" -}}
{{- $internal := include "everruns.internalSecretName" . -}}
{{- $server := include "everruns.name" (list . "server") -}}
- { name: CELL_TOPOLOGY, value: /etc/cell/topology.json }
- { name: CELL_STORE, value: {{ include "everruns.cellStore" . | quote }} }
- { name: CELL_DATA_PLANE, value: local }
- { name: CELL_JWKS_URL, value: {{ printf "http://%s:9000/api/.well-known/jwks.json" $server | quote }} }
{{- /* Nodes deliver the feed there; nodes and executors call its injected
capabilities and Memory there (INJECTED_CAPABILITIES_BASE_URL). */}}
- { name: CELL_CONTROL_PLANE, value: {{ printf "http://%s:9000/api" $server | quote }} }
- name: CELL_DP_TOKEN
  valueFrom: { secretKeyRef: { name: {{ $internal }}, key: EVERRUNS_DP_TOKEN } }
- name: CELL_DP_KEY
  valueFrom: { secretKeyRef: { name: {{ $internal }}, key: EVERRUNS_DP_KEY } }
- { name: RUST_LOG, value: "info,openraft=warn" }
{{- if and .Values.storage.s3.enabled (not .Values.dataPlane.store) }}
{{- with .Values.storage.s3 }}
{{- with .region }}
- { name: AWS_REGION, value: {{ . | quote }} }
{{- end }}
{{- with .endpoint }}
- { name: AWS_ENDPOINT, value: {{ . | quote }} }
{{- if hasPrefix "http://" . }}
- { name: AWS_ALLOW_HTTP, value: "true" }
{{- end }}
{{- end }}
{{- with .existingSecret }}
- name: AWS_ACCESS_KEY_ID
  valueFrom: { secretKeyRef: { name: {{ . }}, key: STORAGE_S3_ACCESS_KEY_ID } }
- name: AWS_SECRET_ACCESS_KEY
  valueFrom: { secretKeyRef: { name: {{ . }}, key: STORAGE_S3_SECRET_ACCESS_KEY } }
{{- end }}
{{- end }}
{{- end }}
{{- with .Values.otel.endpoint }}
- { name: OTEL_EXPORTER_OTLP_ENDPOINT, value: {{ . | quote }} }
{{- end }}
{{- with .Values.dataPlane.extraEnv }}{{ toYaml . | nindent 0 }}{{ end }}
{{- end -}}

{{/* The server's side of the data plane: its default for new organizations and the link. */}}
{{- define "everruns.serverDataPlaneEnv" -}}
- { name: EVERRUNS_DATA_PLANE, value: {{ .Values.dataPlane.version | quote }} }
{{- if include "everruns.dataPlaneRuns" . }}
{{- if eq .Values.dataPlane.version "v2" }}
- { name: EVERRUNS_DEFAULT_DATA_PLANE, value: local }
- { name: EVERRUNS_DEFAULT_DATA_PLANE_URL, value: {{ printf "http://%s-gateway:7000" (include "everruns.name" (list . "cell")) | quote }} }
- { name: INJECTED_CAPABILITIES_BASE_URL, value: {{ printf "http://%s:9000/api" (include "everruns.name" (list . "server")) | quote }} }
{{- end }}
{{- $internal := include "everruns.internalSecretName" . }}
- name: EVERRUNS_DP_TOKEN
  valueFrom: { secretKeyRef: { name: {{ $internal }}, key: EVERRUNS_DP_TOKEN } }
- name: EVERRUNS_DP_KEY
  valueFrom: { secretKeyRef: { name: {{ $internal }}, key: EVERRUNS_DP_KEY } }
{{- end }}
{{- end -}}
