---
type: Specification
title: "Legacy App API Keys"
description: "Frozen execution-only API keys for channel-owned native session ingress."
tags:
  - everruns
  - integrations
---
# Legacy App API Keys

## Status

Existing `api_endpoint` rows can continue to authenticate native session ingress. The keys are frozen compatibility credentials. There is no App API to create, rotate, update, or delete them.

## Channel-owned execution

The canonical routes are channel-scoped:

- `POST /v1/e/{channel_id}/sessions`
- `POST /v1/e/{channel_id}/sessions/{session_id}/messages`
- `GET /v1/e/{channel_id}/sessions/{session_id}`
- `POST /v1/e/{channel_id}/sessions/{session_id}/cancel`

The existing `/v1/apps/{legacy_app_id}/api/{channel_id}/...` forms remain permanent aliases. Alias resolution uses `agent_channels.legacy_app_public_id`; neither route form reads `apps` or `app_channels` while serving traffic.

## Credential and confinement contract

Key material remains represented by the non-secret `api_key_prefix` and the stored SHA-256 `api_key_hash`. Plaintext keys are not stored or returned by archival reads. Authentication uses constant-time hash comparison.

The credential is structurally execution-only. It reaches only the channel session routes and cannot access management APIs. Every read, message, and cancel operation verifies that the target session carries the channel's historical App and channel routing tags. Cross-channel access returns a generic not-found response.

Response projection includes only completed assistant messages and derived task status. Raw tool names, arguments, results, and internal events remain private.

## Lifecycle

Liveness comes from the channel and owning agent. A request is accepted only when the channel is live and enabled, the agent is active, and agent exposures are not suspended. Frozen App status is not consulted.

## Management retirement

The former create, key-regeneration, channel-update, and channel-delete operations are removed from HTTP, OpenAPI, command catalogs, generated clients, and the UI. Existing rows remain unchanged for compatibility.
