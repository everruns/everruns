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

A provider session keeps the agent it was created with. The checkpoint records a digest of the agent definition; a changed definition starts a new provider session on the next turn, and the provider loses the earlier conversation context (the Everruns record keeps it).

## Event projection

The [durable driver](../../crates/host/src/openai_agents_api/durable.rs) projects provider items, not stream events, into the record. Live events drive progress; saved items are authoritative:

| Provider item | Everruns record |
|---|---|
| Assistant `message` (commentary or final answer) | `output.message.started` on first sight, `output.message.delta` from live text, `output.message.completed` with the item's saved text and phase |
| `function_call` / function entry in `required_actions` | Assistant tool-call message, then the Act pipeline's `tool.started` and `tool.completed` |
| `function_call_output` | Nothing new; marks the tool-result outbox delivered |
| `mcp_call` | Assistant tool-call message and `tool.started`, then `tool.completed` (a `failed` status carries the cause in `output`) |
| Root `turn.completed` / `failed` / `cancelled` | Ends the Reason activity; the engine emits `turn.completed` or `turn.failed` from the returned result |

Function and MCP calls are recorded as assistant tool-call messages followed by results, the same transcript shape the native runtime writes, so the record stays replayable and forkable. The backend emits `reason.started` and `reason.completed` around the remote loop. Subagent events (non-null `subagent_id`) are ignored. Unknown progress events are ignored; an unknown `*.failed` or `*.cancelled` event fails closed.

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

Forking into the native runtime starts from the Everruns record; it cannot claim byte-identical hidden context or provider compaction state.

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

**Attribution and metering.** Each Reason emits `capability.usage` for the resolved capabilities and their tools, as the native reason does, and a completed turn emits one `llm.generation` with the turn's usage so budget metering and usage tracking debit the remote spend. Null provider usage debits nothing.

## Observability and cost

Not yet built (EVE-1125). Emit normal Everruns spans from projected turn, generation, tool, and message events, with provider ids as span attributes; projected events already carry `provider_session_id`, `provider_turn_id`, and `provider_item_id` metadata. OpenAI also exposes delayed OTLP trace export; import it only as a linked provider trace. Record root and sub-agent usage separately, upsert late usage, and capture OpenAI tool and container charges, which the `TokenUsage` envelope cannot hold. Native per-call controls inside the Reason loop (provider retry budgets, compaction) do not apply to the provider's internal model calls; the turn reports one generation with the turn's total usage.

## Import to a native agent

Import model, instructions, function schemas, and HTTP MCP definitions. Flag OpenAI built-ins, MCP allowlists, hosted files, environment setup, vault references, multi-agent policy, hidden compacted context, and unsupported transports as explicit warnings. Never silently drop a tool or claim that an imported session is resumable as the same execution. Guarded import, fork, and remote session deletion and retention are EVE-1126; the checkpoint table keeps the provider session id in plaintext for that work.

## Confirmed details and version-sensitive assumptions

Confirmed from OpenAI documentation on 2026-09-29: beta requests use `OpenAI-Beta: agents=v1`; sessions are durable and asynchronous; function calls arrive as required actions and continue through submitted tool results; HTTP MCP can connect from the OpenAI service; root turn completion/failure/cancellation is distinct from session idle; traces can be exported as OTLP JSON; and hosted sandboxes add container charges.

Confirmed against the live API on 2026-10-01: sessions accept and return `metadata`; `GET /agents/sessions` lists newest first and ignores metadata filters; a create repeated with the same `Idempotency-Key` creates a second session; an input event repeated with the same `Idempotency-Key` does not start a second turn; input events reject per-event `id` and `metadata`; a conversation-only session requires initial input; `GET /agents/sessions/{id}/events?stream=true` stays open while idle and delivers live events only; items list with `turn_id`, `order=asc`, and an `after` cursor; the turn resource carries status, error, and usage; a tool result for a turn that is no longer active returns 409; and `DELETE /agents/sessions/{id}` removes a session.

Version-sensitive: exact event and item payloads, usage field names, built-in tool inventory, import/export completeness, environment policy fields, webhook coverage, and whether tool-result submissions honor `Idempotency-Key` as input events do.

## Live validation

On 2026-09-30 the prototype ran end to end against the live API with the dev key: one client function (`lookup_customer`) and one HTTP MCP server (`https://developers.openai.com/mcp`). The recorded stream is `agents_api_live_round_trip.json`, and an earlier call that hit the organization's billing limit is `agents_api_live_failed_turn.json`; both replay through the durable driver in tests.

What the live run taught, beyond the documentation: the preamble and the final answer are separate message items with different phases; MCP completion arrives on `item.done`, not `item.updated`; a failed MCP call has `status: failed`, `error: null`, and the cause in `output`; turn usage was still null at `turn.completed`; and a `usage_limit_exceeded` error event precedes `turn.failed`.

The credentialed conformance test (`live_conformance_one_client_function_and_one_allowed_mcp_tool`, ignored by default, needs `OPENAI_API_KEY`) runs the durable driver with one client function and one MCP server restricted by `allowed_tools`. On 2026-10-01 it reached the API, created a session, and projected the provider's `usage_limit_exceeded` failure, because the organization had no credits left (the Responses API returned `credit_balance_exhausted` at the same time). It has not yet completed a turn.

## Go / no-go

Go for an opt-in, OpenAI-only backend behind the platform flag; no-go as a default or as a replacement for the native runtime. Durable orchestration and policy at the tool and output boundaries are in place. Whether the provider keeps a required action open for as long as an approval may take (15 minutes by default) is unverified against the live API; a provider timeout fails the turn with `tool_action_expired`.

## Follow-up issues

* EVE-1125, observability and cost: spans from projected events, subagent usage, delayed usage upserts, container and tool charges.
* EVE-1126, lifecycle and portability: guarded import to a native agent, fork from the Everruns record, remote session deletion and retention.

## References

- [Agents API overview](https://developers.openai.com/api/docs/guides/agents-api/overview)
- [Sessions](https://developers.openai.com/api/docs/guides/agents-api/sessions)
- [Events and items](https://developers.openai.com/api/docs/guides/agents-api/sessions/events)
- [Function tools](https://developers.openai.com/api/docs/guides/agents-api/tools/functions)
- [MCP connections](https://developers.openai.com/api/docs/guides/agents-api/tools/mcp)
- [Observability and usage](https://developers.openai.com/api/docs/guides/agents-api/observability)
- [Tracing](https://developers.openai.com/api/docs/guides/agents-api/tracing)
