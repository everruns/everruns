---
type: Concept
title: OpenAI Agents API Runtime Backend
description: Mapping, control boundaries, and recommendation for wrapping OpenAI's managed Codex harness.
tags:
  - everruns
  - execution
  - openai
---

# OpenAI Agents API Runtime Backend

## Recommendation

Proceed with a limited OpenAI-only backend, not a default runtime replacement. The API can host the model loop while Everruns remains the control plane, event ledger, approval authority for client functions, cost ledger, and observability exporter. Keep the native runtime as the default because it supports multiple providers and self-hosting and can meet data-retention requirements that the two Agents API environment modes do not currently meet.

The prototype is compile-gated by the `everruns-host/openai-agents-api-prototype` Cargo feature and product-gated by the platform-managed `openai_agents_api` flag (`FEATURE_OPENAI_AGENTS_API`). It does not select the backend in production: nothing in the worker reads the flag yet.

The prototype lives in `crates/host/src/openai_agents_api.rs`. It builds the session config from a resolved `RuntimeAgent`, drives one root turn over the live HTTP API (`run_root_turn`), answers client function calls through a handler, and projects the stream into canonical session events. `crates/host/tests/openai_agents_api.rs` runs it end to end against a mock server with one function tool and one MCP tool, and holds an opt-in live test (`EVERRUNS_OPENAI_AGENTS_API_LIVE=1`).

## Configuration mapping

| Everruns | Agents API | Status |
|---|---|---|
| Resolved model name | `agent.model` | Direct for supported OpenAI models only |
| Composed system prompt and capability instructions | `agent.instructions` | Direct; provider owns later compaction |
| Capability tools and client tools | Function tools in `agent.tools` | Direct schema mapping; Everruns handles required actions |
| HTTP MCP attachments | MCP tools with `server_label`, HTTP `server_url`, and service origin | Direct only after credential scoping and egress review |
| Sub-agent delegation | `multi_agent.enabled` and `max_concurrent_subagents` | Partial; Everruns task identity, policy, and per-child configuration do not map |
| Tool approvals | Pause on function `required_actions` before returning a result | Enforceable for function tools |
| Agent and session files | OpenAI-hosted or self-hosted environment | Partial; mounts, file IDs, checkpoints, and workspace policy differ |
| Network policy | Environment network policy | Partial and version-sensitive |
| Model controls | Agent/session options | Partial; provider-specific controls must be tested per model |

OpenAI built-ins do not cross Everruns' function-result boundary. Everruns cannot promise its hard approval gate or `jev` pre-execution guardrails for web search, command execution, patching, computer use, direct MCP execution, or future built-ins. The first production slice must disable OpenAI built-ins and expose guarded Everruns operations as function tools. Direct MCP is acceptable only for read-only servers or after OpenAI provides an interception contract with equivalent guarantees.

## Event projection

The adapter projects provider events before they reach persistence, SSE, the UI, or SDK:

| Agents API event/item | Everruns event |
|---|---|
| Root `agent.session.turn.created` | `turn.started` |
| Assistant `message` item added / done | `output.message.started` / `output.message.completed`, one Everruns message per item, carrying its `commentary` or `final_answer` phase |
| `agent.session.turn.output_text.delta` | `output.message.delta` on the open message |
| `agent.session.turn.output_text.done` | `output.message.delta` for any text the deltas missed |
| `error` | Nothing; its code and message fill the following `turn.failed` |
| Function entry in `agent.session.requires_action` | `tool.call_requested` |
| `function_call_output` item (the result Everruns submitted) | `tool.completed` for the client function |
| `mcp_call` item added / done | `tool.started`, then `tool.completed`; a `failed` status becomes a failed `tool.completed` with the output text as the error |
| Root `agent.session.turn.completed` | `llm.generation`, `turn.completed` whose final message is the last non-commentary message |
| Root failed or cancelled turn, `agent.session.failed`, `agent.session.environment.failed` | `turn.failed` or `turn.cancelled` |
| `agent.session.idle` after a terminal root turn | `session.idled` |
| Any event whose turn has a non-null `subagent_id` | Nothing; subagents never end the root turn |

The prototype demonstrates one client function, one HTTP MCP tool, and the resulting canonical session events. Provider event IDs, session IDs, and types remain metadata for reconciliation. Unknown progress events are ignored; an unknown `*.failed` or `*.cancelled` event fails closed, and a stream that closes before the root turn reaches a terminal state is an error, not success, because streams do not replay missed events.

OpenAI's `input_tokens` includes cached tokens. Everruns keeps disjoint buckets, so the adapter subtracts `input_tokens_details.cached_tokens`. Null usage stays unknown, never zero.

Gaps: item revisions can arrive after turn completion; usage can lag; sub-agent turns need task identities; command/file events have no exact Everruns equivalent; provider trace spans can duplicate locally reconstructed spans; and text/item ordering needs sequence-based replay tests against live traffic.

## Durability and source of truth

OpenAI is authoritative for the managed loop's live state. Everruns is authoritative for product state, user-visible session events, approvals, guardrail decisions, budgets, and audit history. Store the external session ID and last observed provider cursor in durable turn state. Persist each projected provider event idempotently by provider event ID before acknowledging webhooks or advancing a worker.

On reconnect, retrieve the session, required actions, items, and turns. Reconcile missing provider items into the Everruns log, then resume streaming. A function result is keyed by OpenAI session, turn, and call ID and must be durably recorded before submission. Never repeat an uncertain at-most-once tool call. Forking into the native runtime starts from the imported agent configuration and Everruns message/event record; it cannot claim byte-identical hidden context or provider compaction state.

## Approvals and guardrails

Run existing permission checks, approval policies, `jev` checks, tool hooks, rate limits, and durable result claims before executing a function required action. Return an Agents API tool result only after the normal Everruns decision path settles. This preserves hard gates for Everruns-owned functions.

No equivalent interception exists for direct MCP or OpenAI built-ins in the confirmed documentation. Treat these as outside Everruns enforcement. Do not enable write-capable direct MCP or built-ins for policy-bound agents. Output guardrails can still replace text before Everruns emits `output.message.completed`, but they cannot undo an external side effect the managed harness already performed.

## Observability and cost

Emit normal Everruns spans from projected turn, generation, tool, and message events. Retain provider IDs as span attributes. OpenAI also exposes delayed OTLP trace export; import it only as a linked provider trace, not a second authoritative local trace.

Record root and sub-agent token usage separately when available. Usage can arrive after completion and must be upserted, not assumed zero. Capture model cost, OpenAI tool charges, and container duration/charges. The current `TokenUsage` envelope holds tokens and model cost but not container units, so the prototype preserves the complete provider usage object in event metadata. A production backend needs typed non-token usage and reconciliation against billing exports.

## Import to a native agent

Import model, instructions, function schemas, and HTTP MCP definitions. Flag OpenAI built-ins, hosted files, environment setup, vault references, multi-agent policy, hidden compacted context, and unsupported transports as explicit warnings. Never silently drop a tool or claim that an imported session is resumable as the same execution.

## Confirmed details and version-sensitive assumptions

Confirmed from OpenAI documentation on 2026-09-29: beta requests use `OpenAI-Beta: agents=v1`; sessions are durable and asynchronous; function calls arrive as required actions and continue through submitted tool results; HTTP MCP can connect from the OpenAI service; root turn completion/failure/cancellation is distinct from session idle; traces can be exported as OTLP JSON; and hosted sandboxes add container charges.

Version-sensitive: exact event and item payloads, usage field names, built-in tool inventory, import/export completeness, environment policy fields, webhook coverage, and idempotency behavior. `agents_api_events.json` is a contract sample built from the documented shapes.

## Live validation

On 2026-09-30 the prototype ran end to end against the live API with the dev key (`EVERRUNS_OPENAI_AGENTS_API_LIVE=1`): one client function (`lookup_customer`, answered through the handler) and one HTTP MCP server (`https://developers.openai.com/mcp`). The turn produced a commentary preamble, the function round trip, an MCP search that succeeded and an MCP fetch that failed, and a final answer, all projected into canonical events. The recorded stream is `agents_api_live_round_trip.json` (MCP outputs trimmed). An earlier call that hit the organization's billing limit is recorded as `agents_api_live_failed_turn.json`.

What the live run taught, beyond the documentation: the preamble and the final answer can share one item id with different phases, so each item-added opens a new message; MCP completion arrives on `item.done`, not `item.updated`; a failed MCP call has `status: failed`, `error: null`, and the cause in `output`; turn usage was still null at `turn.completed`, so cost needs a later read of the turn resource; and a transient `usage_limit_exceeded` can fail a turn while other OpenAI APIs still work.

## Go / no-go

Go for an opt-in, OpenAI-only backend behind the platform flag; no-go as a default or as a replacement for the native runtime. It is worth building only if the durability and policy follow-ups land first, because without them a remote loop would bypass Everruns approvals and lose events on a crash.

## Follow-up issues

* EVE-1123, durable orchestration: persist the provider session id, event cursor, and tool-result outbox; reconcile after a restart; select the backend per session behind the flag with a native-runtime fallback.
* EVE-1124, policy at tool boundaries: run approvals, `jev` guardrails, and durable tool claims in the function handler; block write-capable direct MCP and OpenAI built-ins for policy-bound agents.
* EVE-1125, observability and cost: spans from projected events, subagent usage, delayed usage upserts, container and tool charges.
* EVE-1126, lifecycle and portability: guarded import to a native agent, fork from the Everruns record, session deletion and retention.

## References

- [Agents API overview](https://developers.openai.com/api/docs/guides/agents-api/overview)
- [Sessions](https://developers.openai.com/api/docs/guides/agents-api/sessions)
- [Events and items](https://developers.openai.com/api/docs/guides/agents-api/sessions/events)
- [Function tools](https://developers.openai.com/api/docs/guides/agents-api/tools/functions)
- [MCP connections](https://developers.openai.com/api/docs/guides/agents-api/tools/mcp)
- [Observability and usage](https://developers.openai.com/api/docs/guides/agents-api/observability)
- [Tracing](https://developers.openai.com/api/docs/guides/agents-api/tracing)
