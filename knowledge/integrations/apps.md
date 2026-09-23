---
type: Specification
title: "Archived Apps"
description: "Frozen App records and channel-owned ingress compatibility."
tags:
  - everruns
  - integrations
---
# Archived Apps

## Status

Apps are a frozen compatibility domain. They are not a management or deployment surface.
Existing records remain available for history, foreign-key stability, budget attribution, and permanent route aliases.

## Frozen data contract

The following data remains in place and must not be rewritten as part of App retirement:

- all rows in `apps`
- all channel rows in `agent_channels`
- the `app_channels` compatibility view over `agent_channels`
- `sessions.app_id`
- App budget subject values
- reserved session-tag prefixes such as `app:`, `app_channel:`, `slack:app:`, and `ag_ui:app:`

No new App or App-channel write surface exists. Internal compatibility code can read these records for archival responses and historical session behavior.
## Agent channel management

Authenticated channel management lives below the Agent API. It supports listing, creating, reading, editing, deleting, publishing, and unpublishing Agent-owned channels. The Agent Integrations tab provides the same management lifecycle.

New schedules continue to use Agent triggers, which establish the durable workflow binding. A migrated schedule channel remains editable and can run immediately from its Agent-owned channel surface.

## Archival API

Only two authenticated management-plane operations remain:

- deprecated `GET /v1/apps`
- deprecated `GET /v1/apps/{app_id}`

They return frozen records for archival consumers. Create, update, delete, publish, unpublish, channel management, run history, and key rotation operations are retired. App commands are absent from the command inventory and tool catalogs. The `/apps` user interface is removed.

## Channel-owned ingress

`agent_channels` is the source of truth for ingress. Each channel stores its agent ownership, channel identity, channel configuration, authentication data, lifecycle status, and `legacy_app_public_id`.

Traffic-serving resolution reads `agent_channels JOIN agents`. It does not read `apps` or `app_channels`, directly or indirectly. This rule applies to Slack, AG-UI, FCP, A2A, API endpoint, Public Chat, webhook, and schedule compatibility paths.

A channel accepts traffic only when:

- the channel status is `live`
- the channel is enabled
- the owning agent status is `active`
- the owning agent does not have exposures suspended

## Permanent route aliases

Channel-scoped `/v1/e/{channel_id}/...` routes are canonical. Existing App-shaped ingress routes remain permanent aliases. Alias resolution uses `agent_channels.legacy_app_public_id` and channel identity, so it remains deterministic when the frozen App relations are unreadable.

Alias lookups preserve channel-route tenant and error behavior. A mismatched App/channel pair is not found. A channelless legacy alias resolves only when one matching live channel exists; ambiguous matches return the protocol-specific existing ambiguity response.

## Legacy session behavior

Ingress can continue to create sessions with `sessions.app_id` and the reserved App routing tags. Those values are compatibility attribution, not evidence that Apps remain a live management domain. Session ownership and budget attribution keep their existing semantics.

Scheduled proactive execution belongs to Agent triggers. Any grandfathered schedule compatibility path remains channel-owned, supports its existing run-now control, and must satisfy the same no-App-read ingress rule.

## IDs

Frozen App IDs keep the `app_` prefix. Channel IDs keep the `appchan_` prefix.
