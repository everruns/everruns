---
title: Apps Compatibility
description: Understand the retired App model, permanent route compatibility, and the Agent-owned channel model that replaces it.
---

Apps are retired from Everruns management. New integrations belong directly to an Agent as **channels** or **triggers**.

- Use a **channel** when an external peer sends a request and waits for a reply. Slack, AG-UI, A2A, FCP, and Public Chat use channels.
- Use a **trigger** when a schedule or event starts Agent work without a reply channel.

Create and manage both from the Agent's **Integrations** tab.

![Agent Channel Architecture](../images/apps/architecture.svg)

## Existing Apps

Everruns keeps existing App records for historical attribution and compatibility. Existing installs continue to serve traffic, but the App list, detail page, create flow, and management API are retired.

The old `/v1/apps/{app_id}/…` ingress paths remain permanent aliases. They resolve to the migrated channel and continue to work. Do not rewrite a working existing installation only to change its URL.

New integrations use channel-scoped canonical paths:

```text
/v1/e/{channel_id}/slack/events
/v1/e/{channel_id}/ag-ui
/v1/e/{channel_id}/a2a
/v1/e/{channel_id}/fcp
```

## Channel Lifecycle

Each channel has its own lifecycle:

```text
Draft ⇄ Live
Draft → Disabled
Live → Disabled
Disabled → Draft
```

- **Draft**: Configured but does not accept ingress traffic.
- **Live**: Published and able to accept traffic while its Agent is active and exposures are not suspended.
- **Disabled**: Kept for configuration but rejects ingress traffic and does not invoke the Agent.

Publishing or unpublishing one channel does not change another channel on the same Agent.

## Managing Channels

Manage channels from the Agent's **Integrations** tab or through the management API under `/v1/agents/{agent_id}/channels`. Channels were called *endpoints* in earlier releases: the management API moved from `/v1/agents/{agent_id}/endpoints` and the UI from `/agents/{agent_id}/endpoints`. Ingress URLs under `/v1/e/…` did not change, and neither did existing Slack, A2A, or AG-UI installs.

## Where to Go

- [Slack Integration](/integrations/slack/), create and publish a Slack channel.
- [Agent Triggers](/features/agent-triggers/), configure proactive scheduled work.
- [Agent Versions](/features/agent-versions/), select which Agent version a channel uses.
