---
type: Design
title: Bounded Managed Host Dispatch
description: Unmounted host admission, exact supplier request controls and preallocated generation journaling, with explicit authority and supplier qualification gates.
tags:
  - providers
  - security
  - billing
---

# Bounded Managed Host Dispatch

Status: source component, unmounted and unqualified for production. No default
profile, listener, executor capability, credential provisioning or Cloud trial
activation is installed. This is not whole-Cloud spending or tenant-isolation
evidence. Existing [drivers](llm-drivers.md) and legacy journal producers keep
their current behavior.

## Authority stays at the trusted host

The [contract](../../crates/contracts/src/managed_dispatch/mod.rs) separates
caller text from host-selected policy and authenticated context. The trusted
host must resolve the current assignment, effective user/custom policy,
revocation and serving account, then durably bind one preallocated generation
identity, exact request hash and complete financial reservation. It rechecks
authority and wins one durable start before credentials are resolved.

An implementation of the admission trait is trusted application code; it is
not itself a workload identity or authorization proof. Identifier fields are
binding data. A permissive callback must never be mounted as a Cloud adapter.
There is no default implementation and no worker RPC or metadata escape hatch.
The unactivated [assignment-isolation design](https://github.com/everruns/everruns/pull/4987)
and [ingress stack](https://github.com/everruns/everruns/pull/5030) still authorize
journal reads only. This component neither broadens that authority nor replaces
their required issuer, effective-policy provenance and deployment qualification.

## Supplier controls and conditional envelope

The [request compiler](../../crates/contracts/src/managed_dispatch/openrouter.rs)
supports a narrow OpenRouter-to-OpenAI, finite text-only shape. It emits the
supplier route/price/parameter-support controls, completion limit and explicit
tier/cache intent; unsupported caller extensions refuse before admission.
There is one send, no client retry or fallback. The dedicated native transport
disables implicit protocol retries and refuses builder failure without a fallback;
an injected transport must independently meet the same contract. Credentials cannot change the
fixed supplier URL or introduce caller-controlled request headers.

The envelope uses the entire host-qualified model context window, plus the
output ceiling and conservative integer unit-price caps. Request bytes bound
local work; they are not a token estimator or proof of supplier input limits.
This deliberately reserves more than typical usage and releases unused funding
only through the financial owner. Host billing must add its own frozen rates or
margin before admission.

Current primary contracts: [routing and price filters](https://openrouter.ai/docs/guides/routing/provider-selection),
[completion limits](https://openrouter.ai/docs/api_reference/parameters), and
[chat request/routing metadata](https://openrouter.ai/docs/api/api-reference/chat/create-a-chat-completion).
These are conditional request controls, not a universal all-in dollar limit.
No live provider experiment was performed. No production model/account profile
is selected. Qualification must establish the exact endpoint's context and token
semantics, automatic caching, minimums, all fees, retry charging and account
defaults; unsupported semantics must refuse rather than drop a control.

## Receipts, uncertainty and journals

Only the bounded supplier response constructs a [receipt](../../crates/contracts/src/managed_dispatch/receipt.rs).
It carries the original host identity and request hash. Exact decimal cost
parsing rounds upward to nanos without floating-point loss or invented zeroes.
Unexpected routing, hidden tools, missing/malformed usage and excessive cost
require retained review. Missing canonical identity, a failed send, timeout,
oversized response or cancellation leaves the durable start uncertain. None
authorizes a new send, release or fallback debit. Post-send journal errors always
report uncertainty; model output is returned only after the host acknowledges
durable receipt handling. Time/body limits cancel and drop the response reader.

The [journal interface](../../crates/server/src/storage/managed_dispatch/mod.rs)
uses the preallocated generation ID and requires the owner's explicit PostgreSQL
transaction. It checks organization/session/provider attribution and refuses
legacy or changed-observation collisions. It does not fall back to a pool or
become a financial ownership registry. Existing paid/usage history is untouched.
The financial owner must retain request/account binding, canonical supplier
alias uniqueness and settlement/rollups atomically; it may not infer ownership
from an executor event or an already-written journal.

Deleted original provider/session resources do not make completed supplier work
free. The original provider ID remains; a removed session projects as org-only
usage while the admitted host/financial receipt retains its original scope.
The journal projection's lifetime follows existing storage retention; it cannot
replace the permanent financial owner for replay, deletion or reconciliation.

## Qualification and product decisions

- Mounting needs a supported security-owner adapter and durable host state,
  including restart/cancellation recovery and canonical supplier reconciliation.
  The protocol fixtures' in-memory admission is test code, not that adapter.
- Select and qualify the initial model/account/request cohort. Decide whether
  full-context and coarse unit-cap reservations provide acceptable concurrency;
  smaller admission needs a proven supplier input bound, not a local estimate.
- Knowledge retained content, staging and published indexes still require
  finite leases and exhaustion/expiry/detachment/revocation cost-stop behavior.
  Decide retention/removal and who absorbs cost while removal fails or is delayed.
  This text protocol does not implement those services or make them bounded.
- Cloud entitlement pooling, existing-recipient cutover, permanent identity
  retention and audited opening funding remain separate rollout decisions.

[Protocol fixtures](../../crates/contracts/src/managed_dispatch/tests.rs) cover
refusal, wire controls, concurrent/replayed starts, cancellation, unknown send,
bounded readers and canonical review. [PostgreSQL fixtures](../../crates/server/src/storage/managed_dispatch/tests.rs)
exercise real supplier-response parsing into the stable journal interface,
rollback, replay, foreign attribution and deleted-resource liability. Fixtures
use synthetic HTTP and isolated databases, never actual cards or paid inference.

Production/financial activation, deployment, live identity/security policy,
credentials and supplier configuration remain separate review gates. No such
operation is queued by this source component.
