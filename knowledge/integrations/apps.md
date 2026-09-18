---
type: Specification
title: "Archived Apps"
description: "Frozen App records and endpoint-owned ingress compatibility."
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
- all endpoint rows in `agent_endpoints`
- the `app_channels` compatibility view over `agent_endpoints`
- `sessions.app_id`
- App budget subject values
- reserved session-tag prefixes such as `app:`, `app_channel:`, `slack:app:`, and `ag_ui:app:`

No new App or App-channel write surface exists. Internal compatibility code can read these records for archival responses and historical session behavior.

## Archival API

Only two authenticated management-plane operations remain:

- deprecated `GET /v1/apps`
- deprecated `GET /v1/apps/{app_id}`

They return frozen records for archival consumers. Create, update, delete, publish, unpublish, channel management, run history, and key rotation operations are retired. App commands are absent from the command inventory and tool catalogs. The `/apps` user interface is removed.

## Endpoint-owned ingress

`agent_endpoints` is the source of truth for ingress. Each endpoint stores its agent ownership, channel identity, channel configuration, authentication data, lifecycle status, and `legacy_app_public_id`.

Traffic-serving resolution reads `agent_endpoints JOIN agents`. It does not read `apps` or `app_channels`, directly or indirectly. This rule applies to Slack, AG-UI, FCP, A2A, API endpoint, Public Chat, webhook, and schedule compatibility paths.

An endpoint accepts traffic only when:

- the endpoint status is `live`
- the endpoint is enabled
- the owning agent status is `active`
- the owning agent does not have exposures suspended

## Permanent route aliases

Endpoint-scoped `/v1/e/{endpoint_id}/...` routes are canonical. Existing App-shaped ingress routes remain permanent aliases. Alias resolution uses `agent_endpoints.legacy_app_public_id` and channel identity, so it remains deterministic when the frozen App relations are unreadable.

Alias lookups preserve endpoint-route tenant and error behavior. A mismatched App/channel pair is not found. A channelless legacy alias resolves only when one matching live endpoint exists; ambiguous matches return the protocol-specific existing ambiguity response.

## Legacy session behavior

Ingress can continue to create sessions with `sessions.app_id` and the reserved App routing tags. Those values are compatibility attribution, not evidence that Apps remain a live management domain. Session ownership and budget attribution keep their existing semantics.

Scheduled proactive execution belongs to Agent triggers. Any grandfathered schedule compatibility path remains endpoint-owned and must satisfy the same no-App-read ingress rule.

## IDs

Frozen App IDs keep the `app_` prefix. Endpoint IDs keep the `appchan_` prefix.
