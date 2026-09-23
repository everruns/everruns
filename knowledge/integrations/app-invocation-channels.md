---
type: Specification
title: "Legacy App Invocation Aliases"
description: "Frozen App-shaped aliases for channel-owned webhook and schedule ingress."
tags:
  - everruns
  - integrations
---
# Legacy App Invocation Aliases

## Abstract

Webhook and schedule compatibility behavior can retain historical App attribution, but active ingress is channel-owned. New unattended automation uses Agent triggers.

## Webhook ingress

The canonical route is `POST /v1/e/{channel_id}/webhook`. The permanent alias `POST /v1/apps/{legacy_app_id}/webhooks/{channel_id}` remains available.

Resolution and liveness use only `agent_channels JOIN agents`. The alias App ID is matched against `agent_channels.legacy_app_public_id`; traffic serving never reads `apps` or `app_channels`.

Webhook channel configuration retains `token`, `session_mode`, `message`, and optional rate-limit data. Authentication accepts `Authorization: Bearer <token>` or `X-Everruns-Webhook-Token: <token>`.

## Session routing

Invocation channels retain both invocation-keyed `SessionBinding` values:

- `shared_session` reuses one channel-owned session
- `session_per_invocation` creates a new session for each request

Compatibility sessions retain `sessions.app_id` and these reserved routing tags:

- `app:{legacy_app_id}`
- `app_channel:{channel_id}`
- `app_channel_type:{channel_type}`
- `app_invocation:{uuid}` for per-invocation sessions

The `__internal:app_invocation` tag and the `app:`, `app_channel:`, `slack:app:`, and `ag_ui:app:` prefixes remain reserved from external session writes. This prevents a user-created session from impersonating an ingress-owned session or adopting its budget attribution.

## Ownership and isolation

Compatibility sessions keep the channel execution owner and historical App attribution. Shared-session lookup remains scoped by organization, channel identity, owner, and internal routing tags. Mismatched legacy App/channel pairs return the same generic not-found response as canonical channel failures.

## Schedule retirement

New App schedule channels do not exist. Agent schedules use Agent triggers. Grandfathered schedule execution can retain its historical App and channel identity, but its runtime lookup follows the channel-owned no-App-read rule.

## Management surfaces

There is no App invocation management API, App command catalog, or Apps UI. Channel configuration present in frozen rows is read-only through the deprecated archival App reads. Secrets remain redacted in those responses.

## Testing

Regression coverage must prove canonical channel and permanent legacy-alias resolution while both `apps` and `app_channels` are unreadable. Coverage includes every retained channel type, liveness, deterministic alias selection, tenant behavior, and reserved session-tag enforcement.
