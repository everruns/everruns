---
title: Endpoints
description: Expose an Agent to Slack, AG-UI clients, A2A agents, FCP callers, and Public Chat visitors through Agent-owned endpoints, each with its own lifecycle and auth.
appliesTo: [platform, cloud]
---

An **endpoint** is an Agent-owned way for an external caller to reach that Agent and get a reply. Each endpoint belongs to exactly one Agent and has its own transport configuration, authentication, session routing, version policy, and publish state. One Agent can have several endpoints, and publishing or unpublishing one does not change the others.

Use an endpoint when an external peer sends a request and waits for a reply. Use an [Agent trigger](/features/agent-triggers/) when a schedule or event starts Agent work without a reply channel.

![How work enters an Agent: endpoints (Slack, AG-UI, A2A, FCP, Public Chat) route a caller's message to the Agent and return its reply; triggers (schedule, webhook, GitHub, MCP events) send a configured message with no reply channel. Either way the message lands in a session and the turn runs on the Agent's Harness.](../images/features/agent-entry-points.svg)

![Agent Endpoint Architecture](../images/apps/architecture.svg)

## Endpoint types

| Type | `channel_type` | Who calls it | Canonical route | Guide |
|---|---|---|---|---|
| Slack | `slack` | Slack users, through a Slack app | `/v1/e/{endpoint_id}/slack/events` | [Slack](/capabilities/slack/) |
| AG-UI | `ag_ui` | Browser and app clients that speak the AG-UI protocol | `/v1/e/{endpoint_id}/ag-ui` | [AG-UI](#ag-ui) |
| A2A | `a2a` | Other agents, over the A2A protocol | `/v1/e/{endpoint_id}/a2a` | [A2A](/features/a2a/) |
| FCP | `fcp` | Any HTTP client, text in and text out | `/v1/e/{endpoint_id}/fcp` | [FCP](#fcp) |
| Public Chat | `public_chat` | Visitors to a hosted chat website for one Agent | `/v1/e/{endpoint_id}/public-chat` | [Public Chat](#public-chat) |

Routes are relative to the API base, for example `https://your-everruns-host/api`. The endpoint ID in each route is the endpoint's own ID, not the Agent's.

## Create an endpoint

1. Open the Agent and select the **Integrations** tab.
2. Select **Add endpoint** and choose the endpoint type.
3. Configure the type-specific fields and select **Save endpoint**.
4. Select **Publish** to make the endpoint live.

The same operations are available over the API under `/v1/agents/{agent_id}/endpoints`, with `publish` and `unpublish` actions per endpoint. Each endpoint can follow the Agent's default version, its latest version, or a pinned version; see [Agent Versions](/features/agent-versions/).

## Endpoint lifecycle

Each endpoint has its own lifecycle:

```text
Draft ⇄ Live
Draft → Disabled
Live → Disabled
Disabled → Draft
```

- **Draft**: Configured but does not accept ingress traffic.
- **Live**: Published and able to accept traffic while its Agent is active and exposures are not suspended.
- **Disabled**: Kept for configuration but rejects ingress traffic and does not invoke the Agent.

An endpoint serves traffic only when all three hold: the endpoint is live, the Agent is active, and the Agent's exposures are not suspended. Suspending exposures from the Agent's **Integrations** tab takes every endpoint of that Agent offline at once without changing each endpoint's state.

Errors from public endpoints are sanitized so they do not expose internal state, such as whether an endpoint exists or why it is offline.

## AG-UI

An AG-UI endpoint lets a client that speaks the [AG-UI protocol](https://docs.ag-ui.com/) run the Agent and stream its events. The endpoint accepts AG-UI `RunAgentInput` and streams AG-UI events over SSE.

AG-UI endpoints are public client surfaces, so they expose less than the Agent's own event stream:

- `tool_visibility` controls tool activity: `none`, `generic` (a fixed text you configure in `generic_tool_text`), or `narrated`. Raw tool names, arguments, and results are never sent.
- Reasoning summaries, token usage, subagent activity, and client answers to tool-approval interrupts are off by default, each behind its own setting.
- Access is anonymous by default. Set a shared `token` or an `auth` block to require authentication, and `rate_limit_per_minute` to cap requests per IP.
- A thread stays resumable for `session_expiration_seconds` (6 hours by default); after that, the same thread ID starts a new session.

To serve AG-UI from a Rust application without the Platform, see [AG-UI in the Framework](/framework/ag-ui/).

## FCP

An FCP endpoint implements the [Free Communication Protocol](https://github.com/everruns/fcp/blob/main/SPEC.md), a minimal text-in, text-out HTTP interface:

- `GET` returns a Markdown handshake that describes what the endpoint can do and how to authenticate.
- `POST` takes plain text, or `{"message": "..."}`, runs one turn, and returns the Agent's final reply as Markdown. There is no streaming.

Every response, including errors, is `text/markdown` with instructions that point back to the handshake. Access is anonymous by default; set `anonymous` to `false` and a `token` to require a shared bearer token. FCP has its own rate limit and does not accept the `auth` block that AG-UI and A2A use. `response_timeout_seconds` (120 by default) bounds how long a `POST` waits for the reply.

## Public Chat

A Public Chat endpoint serves an isolated, hosted chat website for one Agent. Visitors see only that Agent: there is no console navigation, organization switcher, or access to other agents, endpoints, or sessions.

- Visitors can be anonymous, or sign in through the endpoint's auth configuration, such as Google sign-in. Anonymous visitors can be required to pass a Cloudflare Turnstile challenge before a session starts.
- The chat streams AG-UI events, and raw tool names, arguments, results, and internal IDs never reach the browser.
- The deployment-level `public_chat` feature flag (`FEATURE_PUBLIC_CHAT`) controls whether Public Chat routes are mounted and whether the type appears under **Add endpoint**.

The website reads its public configuration from `/v1/e/{endpoint_id}/public-chat/config`. That response never includes endpoint secrets.

## Other transports

The same endpoint model also carries transports that do not need a reply channel:

- `webhook` runs the Agent when an authenticated HTTP call reaches `/v1/e/{endpoint_id}/webhook`, and is available under **Add endpoint**.
- `schedule` runs the Agent on a cron schedule. Create schedules as [Agent triggers](/features/agent-triggers/), which also cover webhook, GitHub, and MCP event starts.
- `api_endpoint` gives callers an execution-only API key for the session routes under `/v1/e/{endpoint_id}/sessions`.

## Retired Apps

Apps are retired from Everruns management. Everruns keeps existing App records for historical attribution and compatibility, and existing installs continue to serve traffic, but the App list, detail page, create flow, and management API are retired.

The old `/v1/apps/{app_id}/…` ingress paths remain permanent aliases. They resolve to the migrated endpoint and continue to work. Do not rewrite a working existing installation only to change its URL. New integrations use the endpoint-scoped canonical routes above.

## See also

- [Publish an Agent to Slack](/how-to/publish-to-slack/): a step-by-step Slack setup.
- [A2A](/features/a2a/): inbound A2A endpoints and outbound delegation.
- [Agent Triggers](/features/agent-triggers/): proactive scheduled and event-driven work.
- [Agent Versions](/features/agent-versions/): choose which Agent version an endpoint runs.
