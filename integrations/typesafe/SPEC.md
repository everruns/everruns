# TypeSafe Integration

This crate owns the whole TypeSafe surface: the vendor [`client`](src/client/),
the `jev` capability and its tool, the connector catalog entry, and the
deployment classifier that backs guardrail checks. The published
`everruns-host` stays provider-neutral — it no longer depends on TypeSafe at
all, because the platform composes the classifier from above.

## Split

- [`src/client`](src/client/) owns the HTTP
  endpoint, the three question types, the typed answers, retries, and the
  credential-safe error contract.
- This crate owns `capability` (the hosted capability and its tool), `evaluate`
  (the tool protocol), `framework` (the embedder adapter), and `connection`
  (the connector-catalog entry). It also owns `system_judgment`, the deployment
  adapter from the vendor client to `everruns_core::JudgmentService`.

## Capability

`jev` contributes one tool, `jev_evaluate`. The agent supplies the
content and its own typed questions; the tool returns every answer with its
distribution. Score answers additionally carry `normalized`, `level`, and
`label` so the calling code can threshold without re-deriving the level count.

Bounds enforced at the tool boundary, before the network: at most 20 questions,
32 KiB of state, 2000 characters of instructions per question, 20
options/levels per question.

The content being judged is passed as request *state*, never as instructions,
and the tool description and system-prompt addition both say the content is data
to be inspected rather than obeyed.

Credentials resolve from the user's `typesafe` connection first, then the
`TYPESAFE_API_KEY` session secret — the same contract as `brave_search`. The
Framework adapter (`TypeSafe::new`) instead keeps an application-owned
credential inside the client, where it never reaches capability config or
metadata.

The capability is `experimental_only`: dev deployments register it, prod ones do
not.

## Connection

The `typesafe` connector stores an API key. Validation asks the smallest real
question the API accepts — one noul over a two-word state — because there is no
dedicated auth probe; `401`/`403` means the key is invalid, `429` means it is
valid but throttled.

## Relationship to the classifier

This crate owns the whole TypeSafe surface: the vendor [`client`](src/client/),
the `jev` capability and its tool, the connector catalog entry, and the
deployment classifier that backs guardrail checks. The published
`everruns-host` stays provider-neutral — it no longer depends on TypeSafe at
all, because the platform composes the classifier from above.
[`knowledge/operations/judgment-service.md`](../../knowledge/operations/judgment-service.md).

## Tests

- `tests/framework.rs` — the tool through the Framework facade; hosted and
  embedded adapters share one tool protocol.
- `tests/plugin_registration.rs` — inventory registration and deployment gating.

The client's own coverage — wire shape, retries, terminal statuses, credential
safety, pre-flight validation, and the real-API smoke tests behind the
`integration` feature — lives in
[`tests/client.rs`](tests/client.rs).
