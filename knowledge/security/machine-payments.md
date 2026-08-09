---
type: Specification
title: "Machine Payments"
description: "Capability-side payments to external paid services."
tags:
  - everruns
  - security
---
# Machine Payments

## Abstract

Machine payments let Everruns capabilities pay external services during agent
execution without exposing generic paid HTTP access or wallet secrets to the
model. The first supported use case is Parallel's paid search/extract/task API.

V1 deliberately does not add a model-facing `paid_http_request` tool. Payment is
an internal authority that trusted capabilities call while presenting narrow
domain tools such as `parallel_search`.

## Goals

- Let users, agent identities, or organizations fund multiple wallets.
- Let capabilities spend through approved wallets under explicit policy.
- Enforce spending limits before a wallet signs an irreversible payment.
- Record durable attempts and receipts; budget ledger debits are a follow-up.
- Keep wallet credentials out of prompts, chat, session files, tool output, and
  arbitrary shell execution.

## Non-Goals

- No generic model-facing paid HTTP tool in V1.
- No in-chat wallet/key collection.
- No automatic fallback from an unattended agent identity to a user's wallet
  unless a policy explicitly grants that behavior.

## Concepts

### Payment Authority

`PaymentAuthority` is a worker-side service handle available in `ToolContext`.
Capabilities submit structured payment requests with a known operation, URL,
expected price, and body. The authority performs wallet resolution, policy
checks, budget reservation, rail execution, receipt recording, and ledger
posting.

The model never receives private keys, payment challenge headers, payment
payloads, or arbitrary paid-HTTP controls.

External workers must call the control-plane `ExecuteMachinePayment` gRPC
operation rather than executing payment rails locally. This keeps wallet custody,
policy enforcement, signing, and attempt recording in one server-owned trust
boundary.

### Payment Account

A wallet or custodial funding account owned by a principal.

Supported owner types:
- `user`
- `agent_identity`
- `organization`

Each account stores non-secret public metadata plus a separate encrypted
credential/custody reference.

### Payment Policy

A policy grants a subject permission to spend from a payment account under
constraints:
- allowed capabilities (`parallel`)
- allowed hosts (`parallelmpp.dev`)
- max amount per request / turn / day
- approval threshold
- rail preference

Wallet ownership and spend authority are separate. A user may own a wallet but
grant a specific agent identity or app constrained access to it.

### Payment Attempt

Immutable execution record for one paid operation. Attempts include quote data,
request hash, amount, rail, capability, operation, status, receipt metadata, and
non-secret settlement proof references.

### Payment Reservation

A pre-execution budget hold. Unlike LLM token metering, machine payments require
pre-payment enforcement because payments are irreversible. Reservations are
converted to usage ledger debits after settlement, or released on failure.

## Wallet Resolution

Resolution is explicit and conservative:

1. Use the session's `agent_identity_id` wallet if an active policy grants it.
2. Use the initiating user's wallet only for interactive user-initiated turns.
3. Use an organization/operator wallet when delegated to the agent/app/session.
4. If multiple eligible wallets match, use capability config preference.
5. If still ambiguous, fail and ask the user/admin to choose a wallet.

Unattended work must not silently spend a human user's wallet.

## Parallel MVP

The `parallel` capability is the first machine-payment consumer. Core owns only the
trust-boundary primitive (`PaymentAuthority`, payment DTOs, `ToolContext`); the
vendor-specific paid adapter lives in the `integrations/parallel` crate and is
registered as an integration plugin gated by the deployment-controlled `machine_payments` feature
flag. It contributes:
- `parallel_search`
- `parallel_extract`
- `parallel_task`
- `parallel_task_status`

Paid tools call `PaymentAuthority`. `parallel_task_status` is free and polls the
fixed Parallel status endpoint directly.

Known prices:
- search: `$0.01`
- extract: `$0.01` per URL, minimum `$0.01`
- task: `$0.10` for `pro`, `$0.30` for `ultra`

## Rails

V1 rail adapters:
- `x402_base`: native USDC on Base via x402 exact/EIP-3009 payments.
- `mpp_tempo`: reserved rail name; fails closed until a native Tempo/MPP signing
  spec or library is available.

Rails must be native Rust/server adapters. Shelling out to package runners or
wallet CLIs is not allowed. Until a native adapter is configured for a rail, the
authority fails closed after recording the denied attempt.

The authority owns 402 challenge handling and validates amount, recipient,
network, expiry, host, and request hash before signing.

## Security

Threat model entries:
- `TM-AGENT-022`: prompt-injected agents spending money.
- `TM-CRYPTO-008`: wallet credential exposure.

Current mitigations include no generic paid HTTP tool, capability and host
allowlists, per-request caps, encrypted wallet custody, control-plane-only
signing, no worker key exposure, and durable attempt records. Registration of any
money-spending capability, wallet custody UI, and payment account/policy/attempt API is
itself gated by the `machine_payments` feature flag. When disabled, the UI is hidden and
the API routes return a structured `feature_not_enabled` 404, so a deployment that cannot
spend does not collect wallet keys. Budget reservations, per-turn/day
enforcement, and idempotency-key settlement hardening remain follow-ups.

## Rollout

Feature flag: `FEATURE_MACHINE_PAYMENTS` (the deployment-only, API-visible
`machine_payments` feature flag). It is off by default on every grade including dev because
spend is irreversible; set `FEATURE_MACHINE_PAYMENTS=true` to deliberately enable. It is
reported by the feature-flags API so the UI can remove the custody surface, but it is not an
organization opt-in. The flag gates machine-payment capabilities (currently the `parallel`
integration plugin), payment account/policy/attempt APIs, and Settings > Payments.

Recommended sequence:
1. Payment DTOs, account/policy/attempt APIs, and UI.
2. `PaymentAuthority` trait in `ToolContext`.
3. Parallel capability using the authority.
4. Control-plane gRPC payment authority for external workers.
5. Native x402/Base adapter.
6. Native MPP/Tempo adapter when the native protocol is documented.
7. Budget reservation and final ledger posting.
8. Per-turn/per-day limit enforcement.
9. Audit/session timeline payment events.
