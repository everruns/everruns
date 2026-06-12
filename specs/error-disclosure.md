# Error Disclosure

How much detail about run-blocking errors (provider failures, quota
exhaustion, misconfiguration, …) session viewers see, and how driver errors
are classified semantically so that disclosure decisions act on meaning, not
on string parsing.

## Semantic driver errors

LLM drivers classify provider failures at the provider boundary — where the
HTTP status and response body are still available — into `LlmErrorKind`
(`crates/core/src/error.rs`): authentication, quota exhaustion, rate limit,
provider unavailable, invalid request, or other. All drivers (OpenAI chat,
OpenAI Responses, Anthropic, Gemini, Bedrock) attach a kind; `other` falls
back to the legacy string classifier (`classify_runtime_error_message`), so
untyped error paths keep working.

Rationale: string classification conflated distinct operator problems. The
motivating case: OpenAI `insufficient_quota` (out of credits, top up the
account) classified as `provider_misconfigured` — the same code shown for a
bad API key. Quota/billing exhaustion now has its own user-facing code,
`provider_quota_exhausted`, and is treated as non-transient (drivers fail
fast instead of retrying it as a 429).

## Disclosure modes

The `error_disclosure` capability (`crates/core/src/capabilities/error_disclosure.rs`)
selects how the classified error surfaces in the session transcript and
events:

| Mode | Behavior | Intended for |
|---|---|---|
| `generic` | Every blocking error collapses into `processing_error` with no fields — one generic, localizable message | Public-facing agents whose viewers must not learn provider/billing state |
| `standard` | Stable `error_code` + structured `error_fields` (platform default, also applies when the capability is not enabled) | Regular operator-facing sessions |
| `detailed` | `standard` plus a `detail` field carrying the underlying driver error text (truncated) | Trusted surfaces such as coding-agent harnesses built on the runtime; enabled on the built-in `generic` harness |

Per-message override: `controls.error_disclosure` on the session input
message may request a mode, but it is clamped to at most the
capability-configured ceiling (capability absent => `standard`). A client can
narrow disclosure, never widen it (TM-LLM-024).

## Tracking and observability

Disclosure filters only the user-facing layer; internal observability is
unchanged (consistent with `specs/public-endpoints.md` §4):

- `output.message.completed` and `turn.failed` events carry
  `error_disclosure` (the applied mode) alongside the disclosed
  `error_code`/`error_fields`.
- Error message metadata records `error_disclosure` and `source_error_code`
  (the pre-disclosure classification), so generic-mode failures remain
  attributable in analytics.
- Full diagnostic detail (raw provider error strings) stays available to
  operators via `reason.completed` failure events, logs, and tracing in every
  mode; disclosure filters only the user-facing `error_code`/`error_fields`/
  message text.

## Boundaries

- Public endpoints sanitize independently via `PublicError`
  (`specs/public-endpoints.md`); disclosure modes govern the authenticated
  session surface, not the public one. `provider_quota_exhausted` maps to the
  public `service_unavailable`.
- The durable worker's DLQ path (retry exhaustion) currently emits at
  `standard` disclosure because it classifies from persisted task-error
  strings without capability context.
