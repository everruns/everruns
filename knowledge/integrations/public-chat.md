---
type: Specification
title: "Public Chat (Hosted Chat Endpoint)"
description: "Endpoint-owned public chat ingress and isolated visitor experience."
tags:
  - everruns
  - integrations
---
# Public Chat (Hosted Chat Endpoint)

## Abstract

Public Chat serves an isolated chat website for an existing `public_chat` agent endpoint.
The endpoint owns its identity, liveness, authentication, branding, and ingress configuration.
The frozen App record is not part of traffic resolution.

The public surface does not use console authentication or navigation. A visitor can reach only the
agent attached to the resolved endpoint. Errors follow the public-endpoint sanitization contract.

## Lifecycle

Existing Public Chat endpoints remain available under endpoint liveness:

```
live(endpoint) = endpoint.status == live
              && agent.status == active
              && !agent.exposures_suspended
```

App publishing and App-channel management are retired. There is no App or Public Chat builder UI.
Agent Integrations provides read-only endpoint inventory and the agent-level exposure suspension
control.

## Feature flag

The deployment-level `public_chat` flag (`FEATURE_PUBLIC_CHAT`) controls whether Public Chat routes
are mounted. When disabled, the public routes return a sanitized not-found response.

The flag does not replace endpoint liveness. Operators take an existing endpoint offline through
endpoint state or the agent exposure suspension control.

## Addresses

Canonical routes use endpoint identity:

- `GET /v1/e/{endpoint_id}/public-chat/config`
- `POST /v1/e/{endpoint_id}/public-chat`

The permanent compatibility aliases are:

- `GET /v1/apps/{legacy_app_id}/public-chat/config`
- `POST /v1/apps/{legacy_app_id}/public-chat`

Aliases resolve from `agent_endpoints.legacy_app_public_id`. They do not read `apps` or
`app_channels`. Canonical and alias routes apply the same tenant, liveness, authentication,
rate-limit, Turnstile, and error behavior.

The config response contains only public bootstrap data. It never returns endpoint secrets. The
POST route accepts AG-UI `RunAgentInput` and streams AG-UI SSE output.

## Authentication and abuse controls

Public Chat uses the shared endpoint-auth verifier. Existing endpoints can allow anonymous access,
require an optional shared token, or validate configured Google OIDC credentials. Signed-in
visitors bypass the Turnstile challenge. Anonymous visitors satisfy Turnstile when the endpoint
requires it.

Requests pass these gates before agent work starts:

1. Resolve one live endpoint.
2. Authenticate the visitor.
3. Verify Turnstile for anonymous traffic when configured.
4. Apply the Public Chat rate limit.
5. Resolve or create the endpoint-owned session and run the turn.

All failures use sanitized public errors. Responses do not reveal provider details, budget state,
internal identifiers, or the existence of another tenant or endpoint.

## Session and isolation invariants

- Sessions adopt the endpoint owner principal.
- Routing uses endpoint identity while preserving historical reserved App tag prefixes.
- `sessions.app_id` remains frozen compatibility attribution; it is not an ingress lookup key.
- Distinct endpoints and visitors cannot adopt each other's sessions.
- Public Chat cannot enumerate or switch to another agent, endpoint, organization, or session.
- Raw tool names, arguments, results, and internal IDs never reach the public client.

## Public web route

The isolated Next.js surface remains at `/public-chat/{legacy_app_id}` for link compatibility. Its
server calls use the permanent aliases. The route has no console providers, org switcher, or
platform navigation.

## Related specs

- [Apps](apps.md), frozen archival compatibility contract
- [App endpoint authentication](app-endpoint-auth.md), shared inbound authentication
- [App invocation compatibility](app-invocation-channels.md), retained ownership and tags
- [Public endpoints](../execution/public-endpoints.md), ingress aliases and sanitized errors
- [Agent exposure](agent-exposure.md), endpoint liveness and exposure suspension
