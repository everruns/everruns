# TypeSafe Integration

This crate is two layers with one dependency direction.

1. **Client** (`client`, `question`, `answer`, `error`). A standalone Rust SDK for
   TypeSafe's System One endpoint. No Everruns dependency: with
   `default-features = false` the crate is publishable and usable on its own,
   which is why the Everruns pieces are feature-gated rather than the reverse.
2. **Capability** (`capability`, `evaluate`, `framework`, `connection`). The
   `typesafe` capability, its `typesafe_evaluate` tool, the Framework adapter,
   and the connector-catalog entry.

## Client

One endpoint: `POST {base_url}/v1/systemone`. An [`Evaluation`] carries the
state, the model (`jev-latest` by default), and a map of questions keyed by ids
the caller chooses. Ids never reach the model, so each question must carry its
full meaning.

Requests are validated before they are sent — empty question sets, blank
instructions, choice questions with fewer than two options, score questions with
fewer than two levels — because the API rejects all of them and a round trip
costs more than a match arm.

Retry policy: transport failures, `429`, and `5xx` (`529 Overloaded` included)
are retried with doubling backoff, honoring `Retry-After` up to
`MAX_RETRY_AFTER`. `401` and `422` are terminal.

Error bodies are never surfaced verbatim. `Error::Api` carries the status plus
the upstream `message`/`error`/`detail` field, truncated on a char boundary.
An upstream error can echo request headers, and ours carries the API key;
`TypeSafeClient`'s `Debug` omits the key for the same reason.

## Capability

`typesafe` contributes one tool, `typesafe_evaluate`. The agent supplies the
content and its own typed questions; the tool returns every answer with its
distribution. Score answers additionally carry `normalized`, `level`, and
`label` so the calling code can threshold without re-deriving the level count.

Bounds enforced at the tool boundary, before the network: at most 20 questions,
32 KiB of state, 2000 characters of instructions per question, 20
options/levels per question.

Credentials resolve from the user's `typesafe` connection first, then the
`TYPESAFE_API_KEY` session secret — the same contract as `brave_search`. The
Framework adapter (`TypeSafe::new`) instead keeps an application-owned
credential inside the client, where it never reaches capability config or
metadata.

The capability is `experimental_only`: dev deployments register it, prod ones
do not.

## Connection

The `typesafe` connector stores an API key. Validation asks the smallest real
question the API accepts — one noul over a two-word state — because there is no
dedicated auth probe; `401`/`403` means the key is invalid, `429` means it is
valid but throttled.

## Relationship to the judgment service

Everruns' guardrails do **not** depend on this crate. They call
`everruns_core::JudgmentService`, a provider-neutral trait that
`everruns-host` implements on top of this client. That keeps `everruns-builtins`
free of network and vendor edges, the same way `UtilityLlmService` does for chat
completions. See [`knowledge/operations/judgment-service.md`](../../knowledge/operations/judgment-service.md).

## Tests

- `tests/client.rs` — wire shape, retry and terminal-status behavior, credential
  safety, and pre-flight validation, against a mock server.
- `tests/framework.rs` — the tool through the Framework facade; hosted and
  embedded adapters share one tool protocol.
- `tests/plugin_registration.rs` — inventory registration and deployment gating.
- `tests/smoke_real_api.rs` — real API, behind the `integration` feature, keyed
  by `TYPESAFE_API_KEY` from Doppler.
