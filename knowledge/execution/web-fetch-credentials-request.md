---
type: Design
title: "Implementation Request: Secure web_fetch Credentials"
description: "Implementation handoff for bound credential injection, bounded response capture, and secure consent in web_fetch using shared resource access."
tags:
  - everruns
  - execution
  - web-fetch
  - credentials
  - security
---
# Implementation Request: Secure web_fetch Credentials

Status: **Approved for implementation 2026-10-19; qualification in progress.**
This is stage 4 of the scoped resource access plan. The shared ownership/grant
contract is defined in [Scoped Resource Ownership and Access](../security/resource-access.md).
Do not introduce a second credential store or scope resolver to bypass that
dependency. This document describes the requested behavior; implementation
source and exported schemas will own the exact API and field shapes.

## Task

Extend the existing Everruns `web_fetch` tool so an agent can complete an
HTTP authentication flow and call the resulting API without seeing its claim
token, identity assertion, access token, or refresh token. The model handles
references such as `$SHIPMAIL_TOKEN`; the platform handles values.

Use the same credential bindings and access checks as Bashkit and MCP injection.
Support session, agent, virtual-user, and organization owners through the common
services. Scope is not a global fallback chain or proof of runtime authority.

## Current code and prerequisites

Read repository instructions and the following source/knowledge before editing:

- [web_fetch capability and dispatch](../../crates/integrations/src/web_fetch/mod.rs)
- [Raw request parsing, schema, and execution](../../crates/integrations/src/web_fetch/request.rs)
- [FetchKit egress adapter](../../crates/integrations/src/web_fetch/egress_transport.rs)
- [Bashkit bound-secret integration](../../crates/integrations/src/bashkit/http.rs)
- [Current secret contracts](../../crates/contracts/src/runtime/session_services.rs)
- [Current internal secret commands](../../crates/server/src/domains/session_storage/commands/worker/secrets.rs)
- [Host egress contract](../../crates/contracts/src/runtime/egress.rs)
- [Tool approval](tool-approval.md), [Bound Secrets](../security/bound-secrets.md),
  [Virtual Users](../runtime-resources/virtual-users.md), and
  [fetchkit integration](fetchkit.md)

Recheck these paths after the foundation PRs land. Required platform services
are trusted execution context, common resource authorization, write-only
credential resolution/capture, revision-aware storage, and durable consent.
If a host lacks a required service, fail before network I/O. Pure helpers and
test doubles may be developed independently; executable persistent capture
must not ship before its storage and consent enforcement exist.

## Responsibility boundary

| Layer | Responsibility |
| --- | --- |
| Everruns `web_fetch` wrapper | Tool schema, capture declarations, raw-path selection, argument validation, bounded HTTP response processing, JSON pointer extraction, sanitized result construction. |
| Shared credential services | Owner and grant resolution, binding matching, permitted placements, encryption, capture transactions, expiry, revisions, and reusable known-value redaction. |
| Egress boundary | Final network policy, DNS-pinned SSRF protection, transport, and safe metadata-only audit. Injection must occur after destination authorization in the trusted send path. |
| Shared consent API and UI | Durable single-use approval records, authorization checks, secure entry, and rendering the actual capture/binding plan. |
| Upstream FetchKit | Generic fetch/transport primitives only where required; Everruns owners, grants, storage, and consent remain in Everruns. |

Use the existing raw HTTP path for authenticated requests and capture. The
HTML extraction/rendering pipeline must never process an unsanitized capture
response. No upstream FetchKit change is required by the current design;
bounded raw reading can use the existing egress streaming contract. If an
upstream primitive proves necessary, isolate a separate transport-only request
and explain the concrete missing capability.

Refactor reusable matching/redaction into shared helpers rather than copying
Bashkit logic into the tool. `web_fetch` translates its structured headers
and form fields into permitted credential placements; it does not decide
who owns or may use the credentials.

## Required request behavior

1. Accept symbolic credential references in approved bearer headers, exact
   configured custom headers, and exact named form-field values. Form
   substitution happens before form encoding. Do not interpolate arbitrary
   strings in URLs, query strings, raw bodies, or JSON bodies.
2. Resolve every reference against the trusted invocation and the shared
   resource/grant service. Verify the binding's HTTPS origin, port, permitted
   path, auth placement, expiry, revision, and permitted exposure context.
   Reject unknown, unavailable, ambiguous, or unauthorized references before
   sending. Do not retry with another identity.
3. Parse URLs structurally and match paths on valid boundaries. Cover host
   suffix tricks, default ports, URL normalization, encoded path separators,
   and forbidden header/framing fields. Avoid ad hoc string-prefix matching.
4. Preserve existing agent/session and system network policy. Binding does
   not widen network access. Preserve DNS pinning and reject redirects rather
   than forwarding credentials or request bodies to a second destination.
5. A request declaring capture must take the raw path even if it is a GET
   without headers or a body. Reject incompatible crawl, render, extraction,
   and save-to-file options before I/O; do not silently ignore those options.
   Credential requests must not enter an alternate direct-client path.
6. Validate header/body sizes again after substitution. Values must not enter
   tool arguments, narration, approval previews, shell environments, debug
   formatting, persisted request payloads, or tracing attributes.

## Required capture behavior

The tool accepts multiple capture declarations for one response. Each declares
a reference name, a JSON pointer, requested owner scope, permitted destination
and placement, and optional expiry extraction. Owner IDs and grants are
server-derived or resolved through authorized references, never trusted merely
because the model supplied them.

Processing order is mandatory:

1. Validate the complete request/capture plan and obtain any required consent.
2. Authorize and dispatch through egress with no automatic redirect following.
3. Read a bounded response privately. Enforce the cap while reading decoded
   bytes; buffering an arbitrary response and truncating it afterward is not
   sufficient. Bound headers, capture count, pointer depth, and value sizes too.
4. Parse the complete bounded JSON response. Extract all configured values and
   expiry metadata before generating any tool output. Reject ambiguous duplicate
   JSON keys, duplicate names, overlapping pointers, non-string/empty secret
   values, and malformed declarations with sanitized errors.
5. Validate every binding and persist the capture set in one encrypted,
   revision-aware transaction. A failed member cannot leave a partially saved
   access/refresh pair. Preserve an existing credential when replacement fails.
6. Replace captured fields with symbolic references and redact remaining echoes
   from the entire returned object, including headers and URLs. Only then
   construct the public tool result and pass it to hooks, distillation,
   summaries, event recording, or model processing.

Success requires sanitized output and confirmed capture persistence. If JSON
parsing, bounds, extraction, expiry validation, encryption, or storage fails,
omit the response body and upstream diagnostic payload. Never expose the raw
response as a fallback or switch back to FetchKit processing.

Capture normally applies to successful responses. OAuth polling errors such
as `authorization_pending` can be returned through a narrow validated error-code
projection; they must not trigger missing-pointer leakage or expose the raw
error body. Redirects, binary responses, and non-JSON responses never become
successful captures. Validate any retained error code rather than echoing an
arbitrary provider string.

Derive a capture binding from the response issuer's origin by default. The
model may request a narrower same-origin path. A different origin requires a
separate explicit approved destination grant, for example for an identity
assertion issued by one host and submitted to another. Existing network policy
alone does not authorize that transfer.

## Redaction contract

Redact injected values and values captured from this response. Include literal
occurrences plus JSON-escaped and form-encoded representations used by this
feature, with deterministic longest-match handling. Do not skip protection for
short credentials; reject unsupported values or withhold unsafe output instead.

Sanitize headers, body, result metadata, and errors before any model-facing
output. Ensure pre-sanitization response objects and transport errors have safe
debug/log behavior. Do not send raw responses to a model-backed guardrail as a
substitute for deterministic sanitation.

Keep all capture output buffered until validated. Any internal streaming used
to enforce a size cap must not emit response chunks as tool progress or file
content. Preserve the useful non-secret response fields only when sanitation
has completed successfully.

The guarantee covers configured capture fields and known injected values,
including the listed encodings. It does not claim automatic discovery of every
unknown credential or arbitrary server transformation such as reversed or
custom-encoded values. Configure every sensitive field in the Shipmail flow.

## Consent, persistence, and secure entry

Persistent capture at agent, virtual-user, or org scope requires consent or an
existing explicit capture/rotation authorization covering this exact use.
Consent must not depend on enabling the ordinary `tool_approval` capability.
A general approval to call `web_fetch` does not authorize storing credentials.

Before HTTP dispatch, show the source host, destination, scope/owner, names,
placements, expiry policy, consumers, and any replacement. For agent/org
credentials, make shared use clear. User scope requires that person's verified
consent; agent/org scope requires the corresponding management/grant authority.
Session-only capture follows the common owner policy and cannot silently
overwrite or shadow someone else's protected binding.

Bind approval to the final post-hook request/capture plan, actor, org, owning
resource, grant revisions, nonce, and deadline. Store the binding server-side.
Reject replay, retargeting, stale approval, and concurrent double consumption.
Use the existing durable park/resume pattern without holding a worker open.
After resume, revalidate both authority and request identity before dispatch.

Use the common secure-entry flow for manually supplied credentials: the browser
submits directly to the API, never through a message or tool argument. Reuse
shared Settings/credential-management surfaces for metadata and revoke actions;
do not create a `web_fetch`-specific credentials page.

Extract relative expiry using platform receipt time, or validate an absolute
expiry when declared. Reject invalid/overflowing values; do not invent an
expiry when none was supplied. Expose safe expiry/availability metadata to the
model and management UI. Expired/revoked references return clear renewal or
reconnect guidance without borrowing a different grant.

Check revisions immediately before new dispatch and capture commit. Concurrent
revocation must prevent a late capture/refresh from recreating the credential.
Constrained rotation may reuse an explicitly granted flow; it must not widen
issuer, destination, owner, placement, or consumer access.

No exactly-once claim for external POSTs: a timeout/crash can leave upstream
effects uncertain. Deduplicate capture commits by trusted execution identity,
use provider idempotency where supported, and never blindly replay a mutating
authentication request to recover a missing tool result. Already dispatched
requests and disclosed data cannot be recalled by local revocation.

## Out of scope

- A second scope/authorization framework, secret store, or OAuth account model.
- General raw-body/JSON/query substitution or automatic unknown-secret detection.
- Automatic provider-specific refresh scheduling and arbitrary auth-flow execution.
- New GitHub identity or webhook/channel behavior; preserve the recent GitHub
  refactor and use the compatibility adapter stage for common access.
- All memory refactoring, adapters, mounts, ownership, and data migration.
  Memory is deferred to a separate design and rebuild, not another stage of
  this implementation.
- Upstream FetchKit ownership, persistence, UI, or grant APIs.

## Implementation slices

1. Add request/capture parsing, raw dispatch selection, and pure pointer/bounds
   helpers with tests. Keep feature execution gated on required host services.
2. Wire shared credential authorization/injection and deterministic redaction
   into raw HTTP requests. Preserve ordinary FetchKit behavior for non-credential
   requests and existing Bashkit/MCP binding behavior.
3. Add bounded capture, atomic persistence, expiry, sanitized polling errors,
   and revision-safe commit through the common service.
4. Connect durable consent and secure-entry UI. Enable persistent capture only
   once authority checks and the user-facing consent path work end to end.
5. Complete real-flow validation, documentation, exported contracts, and threat
   model coverage. Keep each slice reviewable and avoid parallel legacy writers.

## Acceptance tests and deliverables

Cover both the direct in-process host and remote-worker command path:

- Bearer/custom-header and form injection reaches only the approved destination
  and placement; encoding happens after substitution. Wrong hosts, paths, ports,
  redirects, reserved headers, unauthorized references, and absent services fail
  before dispatch. No credential leaks through URL/error metadata.
- Capture-only GET uses the raw path. Conflicting fetch modes fail before I/O.
  Bounds hold for chunked/decompressed responses, including an oversized secret
  after the limit. No raw chunk is recorded or streamed to model/UI/file output.
- Multiple captures commit atomically. Exercise malformed JSON, duplicate keys,
  missing/overlapping pointers, invalid expiry, very short values, storage and
  encryption errors, and safe OAuth polling. None returns an unsafe body.
- Response fixtures echo injected/captured values in headers, duplicate JSON
  fields, encoded strings, and error text. Scan outputs, persisted events,
  captured tracing, summaries, and file writes for the known test values and
  supported encodings. Duplicate-key capture responses must fail safely.
- Consent is required independently of ordinary tool approval; pending/declined,
  expired, replayed, forged, or retargeted requests cannot send or persist.
  Restart/resume and concurrent answers cannot double-spend one approval.
- Test all four owner scopes through shared conformance fixtures. Alice and Bob
  in one session remain isolated. Wrong responder, other org, Playground, and
  unattended personal access fail under the common contract.
- Revoke between authorization and dispatch/commit, race two rotations, and retry
  after restart. Verify revision checks, atomic persistence, safe uncertainty,
  and no resurrection or implicit reauthorization.
- Run the Shipmail authentication flow with all known token/assertion fields
  configured. A second session and a different agent used by Alice in the same
  org reuse her authorized connection without new connection setup while valid.
  Bob cannot use it. Revoking it in Settings blocks the next authorized send
  with clear reconnect guidance. Session export and trace scans find no values.

Ship source/tests, relevant OpenAPI/SDK/CLI updates where contracts change,
manual UI cases, documentation/changelog, and new or updated threat entries for
each introduced rule. Preserve TM-BASH-030/031/032 and TM-AGENT-038 identifiers;
do not mark residual cases fully mitigated. Run the relevant repository checks
and knowledge validation; the repository workflows own exact commands.

Shipmail response evidence was not available in this workspace when this request
was written. Verify real response pointers, expiry behavior, and cross-origin
exchanges before claiming the real-flow acceptance test passed. Test fixtures
alone do not establish that acceptance result.
