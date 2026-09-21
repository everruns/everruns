---
type: Specification
title: "Public Endpoints"
description: "Public endpoints, error sanitization contract, stable public code set."
tags:
  - everruns
  - execution
---
# Public Endpoints

A **public endpoint** is an HTTP endpoint that accepts unauthenticated traffic and serves it from endpoint-owned, tenant-scoped configuration. Public endpoints have a different threat surface from authenticated management APIs: callers may be anonymous, may be hostile, and have no relationship to operators or owners. This spec defines the contract every public endpoint MUST satisfy.

## Current Public Endpoints

| Endpoint | Route | Source | Notes |
|---|---|---|---|
| AG-UI | `POST /v1/e/{endpoint_id}/ag-ui` | `crates/server/src/api/ag_ui.rs` | Public SSE stream; optional endpoint token |
| AG-UI image upload | `POST /v1/e/{endpoint_id}/ag-ui/images` | `crates/server/src/api/ag_ui.rs` | Public multipart image upload; optional endpoint token |
| FCP handshake | `GET /v1/e/{endpoint_id}/fcp` | `crates/server/src/api/fcp.rs` | Public Markdown handshake (always-open per FCP SPEC) |
| FCP message | `POST /v1/e/{endpoint_id}/fcp` | `crates/server/src/api/fcp.rs` | Public text-in / text-out; optional endpoint token; FCP-only rate limiter |
| Public Chat config | `GET /v1/e/{endpoint_id}/public-chat/config` | `crates/server/src/api/public_chat.rs` | Public non-secret bootstrap (branding, sign-in method, Turnstile site key) |
| Public Chat run | `POST /v1/e/{endpoint_id}/public-chat` | `crates/server/src/api/public_chat.rs` | Public AG-UI SSE stream; anonymous + optional Google sign-in; Turnstile for anonymous; own rate-limiter namespace |
| Slack events | `POST /v1/e/{endpoint_id}/slack/events` | `crates/server/src/api/slack_events/` | Anonymous Slack webhook (signature-verified) |
| Slack manifest | `GET /v1/e/{endpoint_id}/slack/manifest` | `crates/server/src/api/slack_events/` | Anonymous YAML manifest fetch |
| Shared eval run | `GET /v1/public/eval-runs/{token}` | `crates/server/src/api/evals.rs` | Anonymous read-only view of one eval run, gated by an unguessable share token; sanitized DTO (no org/internal/session ids, no internal targets, no attribution env labels); uniform 404 for unknown/revoked/expired |

Any new public endpoint MUST be added to this table and MUST follow the rules below. Existing endpoints that pre-date this contract may not yet route every error path through `PublicError`; aligning them is tracked separately and applies whenever those endpoints stream payload-phase errors to the caller.

The `/v1/e/{endpoint_id}/...` routes are canonical. Existing `/v1/apps/{app_id}/...` routes are permanent aliases. Alias resolution uses the endpoint's frozen `legacy_app_public_id`; traffic serving does not read `apps` or `app_channels`.

## Mandatory Behavior

### 1. Error sanitization

Public endpoints MUST NOT surface internal information to callers:

- No provider names (OpenAI, Anthropic, Gemini, …)
- No model IDs
- No HTTP status codes from upstream calls
- No quota / billing / budget state
- No stack traces, error chains, panic messages, or raw `anyhow` strings
- No internal `user_facing_error_codes` values (these are unstable and provider-leaking)
- No instructions to "contact admin" or "contact support", public users have no relationship to operators

### 2. Stable public error contract

Every **payload-phase / stream-body** error a public endpoint emits MUST come from `crates/server/src/api/public.rs`. The stable public error code set is:

| Code | When | Caller action |
|---|---|---|
| `rate_limited` | Service is busy or upstream-rate-limited | Retry shortly |
| `service_unavailable` | Provider outage, misconfiguration, budget exhaustion, model unavailable, dependency failure | Wait and retry; cannot be self-fixed |
| `request_too_large` | Conversation context exceeds the configured limit | Start a new conversation |
| `internal_error` | **Universal fallback** for anything else, including unknown internal codes and unexpected runtime errors | Retry; if persistent, the operator must investigate |

These four codes are part of the public contract. They MUST NOT be renamed or removed without a deprecation cycle. New codes MAY be added but only when they give callers a *new* actionable distinction; otherwise, fold the case into an existing code.

Pre-stream HTTP errors (4xx malformed-input rejection, generic 500s, generic 404s) are out of scope, see *Non-Goals* below. They MUST still avoid leaking internal state; they just don't carry a public `code` from this set.

### 3. Universal fallback

`internal_error` is the canonical fallback. Every payload-phase code path that emits a public error MUST be reachable from a default branch that produces `internal_error` rather than any internal string. `PublicError::default()` and `PublicError::fallback()` both return this value, and `PublicError::from_internal_code(None)` does as well. When in doubt, fall back, never improvise.

### 4. Internal observability is unchanged

Sanitization happens only at the public boundary. Internal session events (`turn.failed.error`, `turn.failed.error_code`, `turn.failed.error_fields`) still carry full internal detail for operators, audit logs, and tracing. Do not sanitize internal events to avoid losing diagnostic signal.

## Implementation

### Shared module

`crates/server/src/api/public.rs` provides:

- `PublicErrorCode`, enum of the four stable codes
- `PublicError { code, message }`, the wire payload
- `PublicError::from_internal_code(Option<&str>)`, translation from `everruns_core::user_facing_error_codes` to the public set
- `PublicError::fallback()` / `Default`, the universal `internal_error` fallback

Public endpoints adapt `PublicError` into their transport-specific shape (e.g. AG-UI's `RunErrorEvent`) but MUST NOT add fields, codes, or messages beyond what `PublicError` exposes.

### Endpoint adapters

Each public endpoint defines a thin adapter that converts `PublicError` into the transport-specific event:

- AG-UI: `public_run_error_event(error: PublicError) -> AgUiEvent::RunError(...)` in `crates/server/src/api/ag_ui.rs`

When adding a new public endpoint, define one adapter and use it from every error-emitting site, including stream-end / disconnect / cancellation paths. Property tests live alongside `PublicError` in `crates/server/src/api/public.rs` and must continue to pass.

## Threat Model

Public endpoints are the unauthenticated entrypoint for the platform. Relevant categories from `knowledge/security/threat-model.md`:

- **TM-INFO-001 (Information disclosure)**: sanitization above is the primary mitigation.
- **TM-AUTHZ-005**: live-endpoint + active-agent + `anonymous=true` gating before traffic reaches the handler.
- **TM-DOS-010**: per-org / per-session SSE connection limits via the shared SSE tracker.
- **TM-TENANT-009**: routing tags retain the legacy App and endpoint public IDs so compatibility sessions stay isolated.

Any new public endpoint MUST be reviewed against these mitigations before merge.

## Non-Goals

- The HTTP-level error responses returned *before* the streaming/payload phase (for example malformed input, a non-live endpoint, or `not_found()`) describe input shape or public endpoint availability. They keep their specific text when it is actionable and does not leak internal state. They are not routed through `PublicError`.
- Internal sessions, owner-scoped APIs, audit logs, and observability outputs are out of scope. They retain full internal detail.
