---
type: Concept
title: OpenAI Agents API Runtime Backend
description: Mapping, control boundaries, durable orchestration, and recommendation for running an agent's loop on OpenAI's managed Codex harness.
tags:
  - everruns
  - execution
  - openai
  - durability
---

# OpenAI Agents API Runtime Backend

## Recommendation

Proceed with a limited OpenAI-only backend, not a default runtime replacement. The API can host the model loop while Everruns remains the control plane, event ledger, approval authority for client functions, cost ledger, and observability exporter. Keep the native runtime as the default because it supports multiple providers and self-hosting and can meet data-retention requirements that the two Agents API environment modes do not currently meet.

## Selection and enablement

The backend is opt-in at three layers, so the default worker path is unchanged:

1. **Compiled** with the `everruns-host/openai-agents-api` Cargo feature (the worker and server enable it).
2. **Allowed** per org by the platform-managed `openai_agents_api` flag (`FEATURE_OPENAI_AGENTS_API` plus org enrollment). The flag gates the `openai_agents_api_runtime` capability; the server strips gated capabilities from the worker snapshot and rejects them on agent, harness, and session writes.
3. **Selected** per agent or session by that capability, a marker with no tools or prompt ([platform capability](../../crates/platform/src/capabilities/openai_agents_api_runtime.rs)).

The host's Reason activity checks the selection before the native path ([backend wiring](../../crates/host/src/openai_agents_api/backend.rs)). A selected turn falls back to the native loop when its model is not bound to the official OpenAI API (the provider-bound driver exposes its endpoint only for `api.openai.com`) or the host has no durable store. The in-process framework runtime supplies no store, so it always runs natively.

## Configuration mapping

| Everruns | Agents API | Status |
|---|---|---|
| Resolved model name | `agent.model` | Direct for supported OpenAI models only |
| Composed system prompt and capability instructions | `agent.instructions` | Direct; provider owns later compaction |
| Capability, MCP, and client tools | Function tools in `agent.tools` | Direct; every call crosses Everruns' tool pipeline, and MCP credentials stay with Everruns' session-scoped MCP client |
| HTTP MCP attachments as direct MCP | MCP tools with `server_label`, HTTPS `server_url`, `allowed_tools` | Refused by the backend; the protocol layer allows it for custom hosts and the conformance test only with an explicit non-empty allowlist and no credentials |
| OpenAI-hosted tools (`openai_server_tools`, hosted MCP) | Built-in tools | Refused: the turn fails with a policy error instead of dropping or forwarding them |
| Sub-agent delegation | `multi_agent.enabled` and `max_concurrent_subagents` | Refused; Everruns task identity, policy, and per-child configuration do not map |
| Tool approvals and client-side tools | Function `required_actions` held open until a result is submitted | The Everruns turn parks; the provider's required action stays open until the pause is answered |
| Agent and session files | OpenAI-hosted or self-hosted environment | Refused (`environment: none`); text input only, attachments fail closed |

The backend sends Everruns tools only as client functions. Anything else that would run a tool or a model outside Everruns' pipeline (OpenAI built-ins, direct MCP, provider subagents, a hosted environment) is refused before the provider is called ([`ensure_enforceable`](../../crates/host/src/openai_agents_api.rs), [`ensure_runtime_policy`](../../crates/host/src/openai_agents_api/backend.rs)). Everruns cannot promise its hard approval gate, pre-tool guardrails, or network policy for provider-run tools; enabling any of them for policy-bound agents needs an interception contract with equivalent guarantees.

A provider session keeps the agent it was created with. The checkpoint records a digest of the agent definition; a changed definition starts a new provider session on the next turn, seeded from the Everruns record (see [Portability](#portability)).

## Event projection

The [durable driver](../../crates/host/src/openai_agents_api/durable.rs) projects provider items, not stream events, into the record. Live events drive progress; saved items are authoritative:

| Provider item | Everruns record |
|---|---|
| Assistant `message` (commentary or final answer) | `output.message.started` on first sight, `output.message.delta` from live text, `output.message.completed` with the item's saved text and phase |
| `function_call` / function entry in `required_actions` | Assistant tool-call message, then the Act pipeline's `tool.started` and `tool.completed` |
| `function_call_output` | Nothing new; marks the tool-result outbox delivered |
| `mcp_call` | `tool.hosted_call` once in progress and once at its end, with MCP server/tool identity and a fixed safe summary; arguments, output, and errors are never projected |
| OpenAI-hosted call (any other `*_call`, e.g. `web_search_call`) | `tool.hosted_call` once in progress and once at its end |
| `reasoning` | `reason.item` with the provider-curated summary only |
| `compaction` | `context.compacted` with strategy `provider_managed` and unknown (zero) message counts |
| Subagent turn (non-null `subagent_id`) | `tool.hosted_call` named `subagent`, opened at `turn.created` and closed at its terminal event |
| Root `turn.completed` / `failed` / `cancelled` | Ends the Reason activity; the engine emits `turn.completed` or `turn.failed` from the returned result |
| `agent.session.failed`, `agent.session.environment.failed` | Fails the turn (`session_failed`, `environment_failed` when the provider names no code), even if the root turn stays open |

Only client functions enter the Act pipeline and are recorded as assistant tool-call messages followed by results, the same transcript shape the native runtime writes. Provider MCP calls ran inside the managed harness, including its own inventory tools even when no MCP server was configured. Their hosted lifecycle records preserve identity, completion counts, and unknown-price accounting without implying Everruns executed or approved them. The backend emits `reason.started` and `reason.completed` around the remote loop. Every projected event is a type existing SSE, UI, and exporter consumers already render; there is no provider-specific event.

Terminal attribution is strict. A turn terminal event ends the Everruns turn only when it names the root provider turn this Everruns turn adopted: a subagent's terminal closes only its `subagent` record, and a turn terminal that names no turn is ignored until reconciliation reads the turn resource. A subagent's own messages and calls never enter the root transcript, and its required actions are not answered; lifting the subagent refusal needs that mapping and its policy contract too. Unknown progress events and item kinds are ignored; an unknown `*.failed` or `*.cancelled` event fails closed.

OpenAI's `input_tokens` includes cached tokens. Everruns keeps disjoint buckets, so the adapter subtracts `input_tokens_details.cached_tokens`. Usage is read from the turn resource after completion, because it is still null on `turn.completed`; null usage stays unknown, never zero.

## Durability and source of truth

OpenAI is authoritative for the managed loop's live state. Everruns is authoritative for product state, user-visible session events, approvals, guardrail decisions, budgets, and audit history.

One encrypted, lease-fenced checkpoint per Everruns session ([contract](../../crates/core/src/agents_api_store.rs), [PostgreSQL store](../../crates/server/src/storage/agents_api_store.rs), worker RPC `AgentsApiJournal`) holds the provider session id, the create attempt, the agent digest, and the current turn's provider turn id, stream cursor (last applied event id), item correlations, input outbox, tool-result outbox, and final outcome. The lease is per session because one provider session spans turns and accepts one writer. The driver writes the checkpoint ahead of every provider call and local effect:

- **Create.** The create attempt is saved before `POST /agents/sessions`, and the session carries it in `metadata`. The provider does not deduplicate creates, so recovery adopts the session whose metadata carries the attempt instead of creating a second one. If no listed session carries it, the create is treated as never having landed and retried under a new attempt.
- **Input.** Follow-up input is staged with an idempotency key, then sent with `Idempotency-Key`; a retry is a provider no-op. Root turns that existed before the input identify the turn it created.
- **Items.** A correlation moves `open → completing → completed`. After a crash in `completing`, the driver checks the event log before emitting again. Duplicate, replayed, or reordered provider events therefore cannot duplicate local messages.
- **Tool results.** A call is claimed before execution, its result saved before submission, and the submission keyed. A claimed call found after a crash reuses the result already recorded in the event log; an unrecorded one re-enters the Act pipeline, whose durable per-call claim (`durable_tool_results`) decides whether the tool may run again. A 409 on resubmission is treated as delivered; saved `function_call_output` items confirm it.
- **Stream.** Streams do not replay. After a disconnect the driver opens a new stream first, then reconciles from the session (required actions), the turn's saved items, and the turn resource. It reconciles once more before reporting the outcome, so a missing event cannot lose a message.
- **Terminal.** The outcome is saved before the activity returns; a replayed activity returns it without a provider call. Cancellation sends `agent.session.input.cancel` and records a cancelled outcome.

[Restart tests](../../crates/host/tests/openai_agents_api.rs) run the driver against a stateful fake of the API and crash it at each boundary: create, input, function execution, result submission, message emission, and terminal save. Every fake stream ends after the events available so far, so each run also exercises reconnect and reconciliation. Further tests drop or duplicate provider events and replay the two recorded live streams.

A session with a checkpoint cannot be forked; see [Portability](#portability).

## Policy at the tool and output boundaries

The remote loop crosses Everruns policy at two boundaries: every function call the provider requests, and every assistant message it produces (EVE-1124). The [backend](../../crates/host/src/openai_agents_api/backend.rs) supplies the policy; the [durable driver](../../crates/host/src/openai_agents_api/durable.rs) makes each decision durable.

**Function calls.** The calls of one required-action snapshot run as one batch through `execute_act_activity`: permission checks, the `tool_approval` gate, pre- and post-tool hooks (including `guardrails` checks with the `jev` engine), network access, the outbound rate limit, the durable per-call claim, tool narration, and the canonical `tool.started` and `tool.completed`. Before each batch the backend checks the session's budgets; a paused or exhausted budget fails the calls and stops the turn with the canonical budget message. An archived agent or harness stops it the same way.

**Pauses.** The backend decides a pause exactly as the native planner does (`act_pauses_turn`, with the session's client hints). A paused call is saved as parked, the Reason activity returns `waiting_for_tool_results`, and the planner parks the Everruns turn. The provider's required action stays open: nothing is submitted. When the pause is answered, the turn resumes with a later Reason iteration, which resolves each parked call:

| Pause | Resolution on resume |
|---|---|
| Approval gate | The engine-authored `approve_tool_call` result is read back. An approval runs the call again under a fresh local id, recorded as a new assistant tool call, so the gate finds the decision bound to the exact arguments. A rejection, an expiry, or no recorded decision submits a failed result; the call never runs. |
| Client-side tool | The client's recorded result is submitted. |
| Connection setup, URL or form elicitation | The call runs again under a fresh local id. |

A replay of the activity that parked (the same iteration) stays parked. A call that keeps asking for a user action fails after three attempts. If the provider ends the turn while a call is parked (its own timeout or a cancel), the Everruns turn fails with `tool_action_expired`. A new message that abandons a parked turn cancels the provider turn before its input is sent.

Unlike the native loop, the approved call runs without a model retry: the provider is still waiting on the original call, so the gated call's `tool_approval_required` placeholder never reaches it.

**Assistant messages.** Streaming output guardrails run on each message's live text; once one trips, no more of that message reaches the client. End-of-message guardrails (moderation, LLM judges, `jev`) withhold live text and judge the completed message. Every completed message is judged again on its saved text, so a missed stream cannot skip the check. A trip saves a policy stop before any effect, emits `output.message.replaced` and the replacement as the canonical message, cancels the provider turn, and completes the Everruns turn with the replacement, as the native loop ends a turn whose output was replaced. The provider still holds the original text in its own session; a guardrail cannot undo an external side effect the harness already performed.

**Attribution and metering.** Each Reason emits `capability.usage` for the resolved capabilities and their tools, as the native reason does. Accounting is described under [Observability and cost](#observability-and-cost).

## Observability and cost

The [projection](../../crates/host/src/openai_agents_api/durable/observe.rs) keeps Everruns ids as each event's identity and records the provider's beside them in event `metadata` ([correlation keys](../../crates/core/src/events/correlation.rs)): runtime backend, provider session, turn, item, subagent, and the session's trace export URL (`GET /agents/sessions/{id}/traces`, OTLP JSON). The OpenTelemetry listener copies them onto every span as `everruns.provider_*` attributes and the Braintrust listener into span metadata, so spans are built from the canonical events by the existing exporters. The provider trace is linked, never fetched or imported.

**Accounting.** Every provider turn that ends (completed, failed, cancelled by Everruns or the provider, or stopped by policy) is billed once, by the durable driver, as one `llm.generation` keyed to the provider turn by `response_id`: the root turn, and each subagent turn under it separately. A turn that did not complete is a failed generation that still carries what it spent. Usage is read from the turn resource and re-read a few times while it is still null, because the provider fills it late. The generation's `cost_components` ([`LlmCostComponent`](../../crates/core/src/events/llm_data.rs)) list tokens priced from the model profile, OpenAI-hosted calls priced per call where a price exists, and a hosted container whenever the turn used one. An amount nobody can price is a component with no `cost_usd`, never a zero; budgets debit the priced components and journal the unknown ones (`cost_unknown_components`, see [budgeting](../security/budgeting.md)). The turn reports root and subagent usage summed, or unknown when any part is.

**Late usage** (EVE-1145). The re-reads are bounded, so usage the provider fills later still arrives as an unknown. Such a generation is recorded in `llm_generations` as `usage_pending`, with zero tokens, its priced components as the cost, and what reads it back: the provider turn (`provider_response_id`), the provider session, and the Everruns provider whose credentials ran it (`everruns_provider_id` on the event's `metadata`). A periodic [reconciler](../../crates/server/src/services/agents_api_usage.rs) (every `AGENTS_API_USAGE_RECONCILE_INTERVAL_SECS`, default 60, 0 disables) re-reads the turn with that provider, only through the official API (TM-LLM-043), and once usage is there writes the real token counts and price-table cost to the record, adds the late tokens to the session and agent totals, and debits them to budgets. Nothing already debited is charged again: the late debit covers only the tokens. No generation is debited twice: the late journal entry is keyed to the generation record, which the journal's unique source index admits once, and the record update is guarded on `usage_pending`, so exactly one pass applies the totals. A turn still without usage after a bounded number of reads (about an hour) stays an explicit unknown.

The accounting record is written ahead like the transcript, but the event log cannot be searched for it, so a crash between the save and the emit loses that one record rather than billing it twice. Replays, duplicate or missing stream events, reconnects, and restarts bill each provider turn once ([contract tests](../../crates/host/tests/openai_agents_api_observability.rs)).

Hidden reasoning stays with the provider: a `reasoning` item's `content` and `encrypted_content`, and a `compaction` item's encrypted context, are never read (TM-LLM-034). Native per-call controls inside the Reason loop (provider retry budgets, compaction) do not apply to the provider's internal model calls.

Version-sensitive: the Agents API documents a turn's `subagent_id` but no parent turn field; the driver attributes a subagent turn to the root turn it saw it under, or to a root named by `parent_turn_id`/`root_turn_id` when the provider sends one. It assumes root and subagent usage are reported separately, and the `compaction` and hosted-call item shapes follow the Responses API.

## Session lifecycle

The Everruns session is the product object. The provider session is loop state Everruns creates, reuses, and deletes on its behalf (EVE-1126): [driver](../../crates/host/src/openai_agents_api/durable.rs), [failure classification and deletion](../../crates/host/src/openai_agents_api/lifecycle.rs), [deletion and retention task](../../crates/server/src/agents_api_lifecycle.rs), [tombstone trigger](../../crates/server/migrations/156_agents_api_session_lifecycle.sql).

**Creation.** A provider session is created lazily, by the first turn routed to the backend, with that turn's provider credentials. The checkpoint row keeps the provider session id and the Everruns provider that owns it (`provider_key`) in plaintext, so lifecycle work never decrypts the checkpoint and never stores a credential. A changed agent definition or a turn on another Everruns provider starts a new provider session; the replaced one is queued for deletion. A new provider session is seeded with the recent conversation from the Everruns record (see [Portability](#portability)).

**Cancellation.** Cancelling an Everruns turn sends `agent.session.input.cancel` and saves a cancelled outcome; the provider session stays for the next turn. A new message that abandons a parked turn cancels that provider turn first.

**Deletion.** Every path that drops a provider session id records a tombstone in the same transaction: deleting the session, any cascade into the checkpoint (agent, harness, or organization deletion), replacing the provider session, and releasing it. A server task, running in both runtime modes, claims due tombstones with leased `SKIP LOCKED` claims (safe across replicas), resolves the owning provider's current key, and calls `DELETE /agents/sessions/{id}` on the official API only. Success or 404 removes the tombstone, so a retried deletion converges. Other failures back off exponentially and record a stable code, never a response body. A tombstone whose provider no longer exists, or that keeps failing, stays as `failed` for operators. Deleting a session mid-turn removes the checkpoint, which fences its worker, and the provider deletion stops the remote loop.

**Retention.** `AGENTS_API_SESSION_RETENTION_DAYS` (server, off by default) releases provider sessions whose checkpoint has been idle that long: unleased, the Everruns session `started` or `idle`, and no call parked on an approval or client result. The release is queued for deletion like any other; the Everruns record stays, and the next turn starts a new provider session. `EVENT_RETENTION_DAYS` archives Everruns events only.

**Credential rotation.** Turns and deletions resolve the provider's key when they run, so rotating the key of the same Everruns provider (the same OpenAI project) keeps the provider session. Moving a key to another OpenAI project under the same Everruns provider hides the old session: the next turn fails once with `provider_session_unavailable`, and its deletion resolves as already gone, leaving that session to OpenAI's retention. Rotate within a project, or delete the Everruns sessions first. Rotating the Everruns encryption key re-encrypts the checkpoint with the other registered columns.

**Unavailability.** A provider failure no retry fixes ends the turn with a stable code and an Everruns-authored message; provider response bodies, which can echo part of a key, never reach the turn. The codes pass the session's error-disclosure ceiling like native failures.

| Condition | Outcome |
|---|---|
| Credentials rejected (401) | Turn fails `provider_misconfigured`; the provider session is kept |
| Preview access withdrawn or not enabled (403, or 404 on create) | `provider_misconfigured`; the provider session is kept and resumes when access returns |
| Model unavailable (create rejected, or a turn failure naming the model) | `model_unavailable` |
| Account out of credits or quota | `provider_quota_exhausted` |
| Provider session gone (404, confirmed by reading the session) | `provider_session_unavailable`; the session is released and the next message starts a new one |
| Rate limit, 5xx, network | Activity error; the durable engine retries |
| MCP server unreachable | MCP tools are client functions through Everruns' MCP client, so the call fails as a tool result, as in the native runtime, and the turn continues |
| `openai_agents_api` flag turned off for the org | The capability is stripped; turns run natively from the Everruns record; the provider session stays until deletion or retention releases it |

## Portability

**Export.** The Everruns record (session events and messages) is the export; there is no separate one. It reconstructs every user input, every assistant message as the provider saved it after Everruns output guardrails, every client function call and result, provider MCP and OpenAI-hosted lifecycle markers, reasoning summaries, compaction markers (`context.compacted`, strategy `provider_managed`), and usage and cost. It cannot reconstruct the context the provider works from after managed compaction (encrypted, never read), hidden reasoning, provider subagent transcripts, or the provider's internal model calls. Replaying the record natively reproduces the conversation, not the provider's working context.

**Seeding** (EVE-1146). A turn that creates a provider session (the first one, or one replacing a session that was replaced, released by retention, or lost as `provider_session_unavailable`) sends a transcript of the earlier turns ahead of its own input ([seed](../../crates/host/src/openai_agents_api/seed.rs)). It is built from the turn's assembled history (what the native loop would see, after Everruns' own context filtering) and carries user text, assistant text as recorded after output guardrails, and completed tool call/result pairs; a call without a result, system messages, attachment bytes (counted, not sent), reasoning (readable or encrypted), and provider-native opaque content are never sent (TM-LLM-034). Session-create `input` takes user-role messages only, so the transcript cannot be typed `assistant` or `function_call` items: it is one user message, a framing header plus a fenced block of JSON lines with `<` and `>` escaped so no entry can close the fence or forge another (TM-LLM-046), followed by the turn's text as a second user message. The bound keeps the newest entries: at most 200 entries and 32 KiB of transcript, each text, argument, or output clipped to 4 KiB, with the omitted count stated. An existing provider session already holds the conversation and gets no transcript, and an uncertain create is adopted, so the seed is never sent twice. What a seed cannot restore is the same as for export: the provider's post-compaction context, hidden reasoning, and subagent transcripts. The new session sees the conversation as the record kept it, flattened into one user message; the model may still call a tool again rather than trust a recorded result.

**Fork.** Refused at the API boundary with `409` and code `agents_api_session_not_forkable` for any session with a checkpoint, including detached spawns seeded as a fork: a fork would continue without the provider-held context while presenting itself as a copy. Workspace-only seeds copy no conversation and are allowed.

**Import.** No endpoint imports a provider session as an Everruns session: the provider cannot hand over its compacted context, so an import could not resume as the same execution. `import_session_config` maps an Agents API agent definition to a native one and reports OpenAI built-ins, MCP allowlists, hosted files, environment setup, multi-agent policy, and unsupported transports as explicit warnings; it never silently drops a tool or carries MCP credentials.

## Self-hosting and data residency

OSS builds compile the backend, but it runs only with the platform flag, the capability, an OpenAI provider on `api.openai.com`, PostgreSQL, and an encryption key; without them sessions run natively. Selecting it moves the loop's working state to OpenAI: instructions, user input, tool results, assistant output, reasoning, and compacted context live in the provider session, under the key's OpenAI project and its data controls and region, not only in the self-hosted database. Zero Data Retention is not available for the environment modes (see [Recommendation](#recommendation)). Everruns deletes provider sessions it no longer references and can release idle ones, but cannot verify OpenAI's own deletion or backups, and cannot delete a session once the credentials that own it are gone: delete Agents API sessions before deleting their provider or organization. Deployments whose residency requirements OpenAI does not meet should keep the native runtime.

## Confirmed details and version-sensitive assumptions

Confirmed from OpenAI documentation on 2026-09-29: beta requests use `OpenAI-Beta: agents=v1`; sessions are durable and asynchronous; function calls arrive as required actions and continue through submitted tool results; HTTP MCP can connect from the OpenAI service; root turn completion/failure/cancellation is distinct from session idle; traces can be exported as OTLP JSON; and hosted sandboxes add container charges.

Confirmed against the live API on 2026-10-01: sessions accept and return `metadata`; `GET /agents/sessions` lists newest first and ignores metadata filters; a create repeated with the same `Idempotency-Key` creates a second session; an input event repeated with the same `Idempotency-Key` does not start a second turn; input events reject per-event `id` and `metadata`; a conversation-only session requires initial input; `GET /agents/sessions/{id}/events?stream=true` stays open while idle and delivers live events only; items list with `turn_id`, `order=asc`, and an `after` cursor; the turn resource carries status, error, and usage; a tool result for a turn that is no longer active returns 409; and `DELETE /agents/sessions/{id}` removes a session.

Confirmed against the live API on 2026-10-02: session-create `input` accepts an array of messages, but only with role `user` (an `assistant` message returns `invalid_value` on `input[n].role`, and a `function_call` item returns `missing_required_parameter` for its `role`); `{"type": "message", "role": "user", "content": [{"type": "input_text", "text": ...}]}` works, and the provider folds several user messages into one saved user item with one `input_text` part each, which the driver does not project. A seeded session answered from the transcript in the same turn.

Version-sensitive: exact event and item payloads, usage field names, built-in tool inventory, import/export completeness, environment policy fields, webhook coverage, and whether tool-result submissions honor `Idempotency-Key` as input events do.

## Live validation

On 2026-09-30 the prototype ran end to end against the live API with the dev key: one client function (`lookup_customer`) and one HTTP MCP server (`https://developers.openai.com/mcp`). The recorded stream is `agents_api_live_round_trip.json`, and an earlier call that hit the organization's billing limit is `agents_api_live_failed_turn.json`; both replay through the durable driver in tests.

What the live run taught, beyond the documentation: the preamble and the final answer are separate message items with different phases; MCP completion arrives on `item.done`, not `item.updated`; a failed MCP call has `status: failed`, `error: null`, and the cause in `output`; turn usage was still null at `turn.completed`; and a `usage_limit_exceeded` error event precedes `turn.failed`.

The credentialed conformance test (`live_conformance_one_client_function_and_one_allowed_mcp_tool`, ignored by default, needs `OPENAI_API_KEY`) runs the durable driver with one client function and one MCP server restricted by `allowed_tools`. On 2026-10-01 it reached the API, created a session, and projected the provider's `usage_limit_exceeded` failure, because the organization had no credits left (the Responses API returned `credit_balance_exhausted` at the same time). On 2026-10-02, with a funded key, it completed: one `lookup_customer` call, one allowed MCP search, and a final answer. `live_seeded_session_recalls_the_earlier_conversation` seeds a session with an earlier exchange (a code word and a tool result) and checks the answer recalls both; it passed the same day. Both runs showed the managed harness calling its own `codex` MCP resource listings (`list_mcp_resources`, `list_mcp_resource_templates`) with no MCP server configured and `environment: none`; they are provider `mcp_call` items projected as `tool.hosted_call` records.

## Go / no-go

Go for an opt-in, OpenAI-only backend behind the platform flag; no-go as a default or as a replacement for the native runtime. Durable orchestration, policy at the tool and output boundaries, the event, usage, and cost projection, and the session lifecycle are in place. Whether the provider keeps a required action open for as long as an approval may take (15 minutes by default) is unverified against the live API; a provider timeout fails the turn with `tool_action_expired`.

## Follow-up issues

* The managed harness's own `codex` MCP resource-listing calls (observed live on 2026-10-02) run outside Everruns' tool pipeline; confirm what they can reach and whether the configuration can turn them off.
* An uncertain create whose Everruns session is deleted before the next turn adopts it leaves that provider session to OpenAI's retention.

## References

- [Agents API overview](https://developers.openai.com/api/docs/guides/agents-api/overview)
- [Sessions](https://developers.openai.com/api/docs/guides/agents-api/sessions)
- [Events and items](https://developers.openai.com/api/docs/guides/agents-api/sessions/events)
- [Function tools](https://developers.openai.com/api/docs/guides/agents-api/tools/functions)
- [MCP connections](https://developers.openai.com/api/docs/guides/agents-api/tools/mcp)
- [Observability and usage](https://developers.openai.com/api/docs/guides/agents-api/observability)
- [Tracing](https://developers.openai.com/api/docs/guides/agents-api/tracing)
