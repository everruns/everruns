---
title: Apps Compatibility
description: Understand the retired App model, permanent route compatibility, and the Agent-owned endpoint model that replaces it.
---

Apps are retired from Everruns management. New integrations belong directly to an Agent as **endpoints** or **triggers**.

- Use an **endpoint** when an external peer sends a request and waits for a reply. Slack, AG-UI, A2A, FCP, and Public Chat use endpoints.
- Use a **trigger** when a schedule or event starts Agent work without a reply channel.

Create and manage both from the Agent's **Integrations** tab.

![Agent Endpoint Architecture](../images/apps/architecture.svg)

## Existing Apps

Everruns keeps existing App records for historical attribution and compatibility. Existing installs continue to serve traffic, but the App list, detail page, create flow, and management API are retired.

The old `/v1/apps/{app_id}/…` ingress paths remain permanent aliases. They resolve to the migrated endpoint and continue to work. Do not rewrite a working existing installation only to change its URL.

New integrations use endpoint-scoped canonical paths:

```text
/v1/e/{endpoint_id}/slack/events
/v1/e/{endpoint_id}/ag-ui
/v1/e/{endpoint_id}/a2a
/v1/e/{endpoint_id}/fcp
```

## Endpoint Lifecycle

Each endpoint has its own lifecycle:

```text
draft → live → draft
```

- **Draft**: Configured but does not accept ingress traffic.
- **Live**: Published and able to accept traffic while its Agent is active and exposures are not suspended.
- **Disabled**: Kept for configuration but does not invoke the Agent.

Publishing or unpublishing one endpoint does not change another endpoint on the same Agent.

## Where to Go

- [Slack Integration](/integrations/slack/), create and publish a Slack endpoint.
- [Agent Triggers](/features/agent-triggers/), configure proactive scheduled work.
- [Agent Versions](/features/agent-versions/), select which Agent version an endpoint uses.
