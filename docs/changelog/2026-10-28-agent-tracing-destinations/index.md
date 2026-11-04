---
title: Route agent traces to your own destinations
description: Connect multiple Braintrust projects and OTLP collectors, share organization defaults, and override trace routing for individual agents.
date: 2026-10-28
cloud: 2026-10-28
kind: feature
significance: notable
products: [platform, cloud]
release: "0.50.1"
surfaces: [ui, api, cli]
links:
  docs: [/observability/destinations/]
---

Connect multiple Braintrust projects and OpenTelemetry collectors in
**Settings → Organization → Tracing**. New destinations start unassigned and
export metadata only. Credentials stay encrypted; messages, tool content, and
reasoning require explicit opt-in.

Choose organization defaults, then use an agent's **More → Tracing** drawer to
inherit, add destinations, replace defaults, or turn customer exports off.
Routing changes apply to new turns. Personal Chat stays excluded, and existing
platform telemetry continues independently. Send a synthetic test trace and
inspect delivery status without running an agent.

Stop one agent's customer exports from the CLI:

```sh
everruns tracing routing update --id agent_01933b5a000070008000000000000001 \
  --mode off --destination-ids '[]' --reason 'Disable external exports'
```

![Organization tracing destinations with defaults and agent selections](./organization-tracing@2x.png)

![Agent tracing drawer selecting a Braintrust destination](./agent-tracing@2x.png)

Read the [setup and delivery guide](/observability/destinations/).
