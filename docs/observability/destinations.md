---
title: Customer tracing destinations
description: Connect organization and agent traces to multiple Braintrust projects and OpenTelemetry collectors with explicit privacy and routing controls.
---

Connect your own tracing backends from **Settings → Organization → Tracing**.
Organization administrators manage destinations and routing. Members can inspect
non-secret destination metadata and routing.

Self-hosted deployments must configure `SECRETS_ENCRYPTION_KEY` before creating
connections. See [Environment variables](/sre/environment-variables/). Collector
hosts must also be allowed by the deployment's egress policy.

## Add a destination

1. Choose **Add destination** and give it a recognizable name.
2. Select **Braintrust** or **OpenTelemetry / OTLP**. You can add multiple
   destinations using the same provider.
3. Enter an HTTPS endpoint. For Braintrust, enter the project UUID and API key.
   For OTLP, choose the authentication header your collector expects, or leave
   it empty for a collector that needs no authentication. Supported headers are
   `Authorization`, `X-API-Key`, and `Api-Key`. The exporter uses
   HTTP/protobuf and appends `/v1/traces` unless it is already present.
4. New destinations start with metadata only. After creation, use **Edit destination**
   to enable **Include messages and tool content** when your retention policy
   permits exporting inputs, outputs and tool arguments/results. **Include reasoning content**
   requires a separate opt-in.
5. Save, then use **Send test trace** from the destination menu. The test
   contains synthetic metadata. **Export accepted** means the receiver accepted
   the request, not that the provider has indexed it in its UI.

Credentials are encrypted and never returned by the API. Editing a destination
leaves stored credentials intact unless you replace authentication. Changing the
receiver requires new authentication settings. Endpoints cannot contain URL
credentials, queries or fragments. Host egress policy can block private networks
or disallowed hosts; customer exports require DNS pinning.

New destinations start **Unassigned**. Creating one does not export agent data.

## Choose routing

Use **Edit defaults**, select destinations and **Save changes** to set organization
defaults. From an agent's **More → Tracing** drawer choose:

- **Use organization defaults**: use the organization's current defaults.
- **Add to organization defaults**: send to defaults plus the selected destinations, deduplicated.
- **Use selected destinations**: send only to the selected destinations.
- **Turn off external tracing**: stop customer exports for this agent.

You can select up to eight defaults and eight destinations in an agent policy.
Each organization can configure up to 32 destinations. Changes apply to new turns,
including new turns in existing sessions. A running turn keeps its initial routing.
Guest responders use their own agent policy; child sessions resolve their own
policy. Personal Chat and sessions descended from it are excluded.

**Discard** abandons unsaved routing edits. **History** reviews saved routing;
restores use revision checks. A stale Save or Undo requires reloading and reviewing
the current configuration. Page-side Chat can draft routing for manual review;
credentials and content-capture settings remain protected manual controls.
Destination history restores its name and pause state while preserving current
credentials, receiver, and content settings.

## Delivery and troubleshooting

Exports use independent, bounded queues and retries. A slow or failing receiver
does not stop agent execution or other destinations. Delivery is best effort:
queue overflow, exporter limits, process restart or a routing lookup failure can
lose events. A queued turn-start is omitted if its organization or agent policy changed
after the turn began; destinations configured later are also excluded. New consent never
applies backward to delayed starts. There is no historical replay or guaranteed delivery. After an exporter restart,
already-running turns are omitted rather than re-routed under newer settings. The table shows
receiver acceptance, safe failure messages and destination queue drops. Global
router drops also have a server metric, `everruns_customer_tracing_router_dropped`.
Enabled destinations with delivery failures contribute an organization health issue.

**Pause destination** prevents subsequent sends, including queued retries, at the
network boundary. Changing configuration or authentication also retires queued
sends using the previous revision. An already in-flight request cannot be recalled.
Remove a destination from all policies before deleting it; deletion leaves data
already exported to the provider intact.

Deployment-wide [OpenTelemetry](/observability/opentelemetry/) and
[Braintrust](/observability/braintrust/) configuration continues independently.
Customer settings do not change platform exporters.
