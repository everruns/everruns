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

{{- define "everruns.natsUrl" -}}
{{- if .Values.nats.url -}}
{{- .Values.nats.url -}}
{{- else if .Values.nats.enabled -}}
{{- printf "nats://%s:4222" (include "everruns.name" (list . "nats")) -}}
{{- end -}}
{{- end -}}

{{- define "everruns.valkeyUrl" -}}
{{- if .Values.valkey.url -}}
{{- .Values.valkey.url -}}
{{- else if .Values.valkey.enabled -}}
{{- printf "redis://%s:6379" (include "everruns.name" (list . "valkey")) -}}
{{- end -}}
{{- end -}}

{{/* Stop with a clear message before anything is created. */}}
{{- define "everruns.validate" -}}
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
{{- end -}}
