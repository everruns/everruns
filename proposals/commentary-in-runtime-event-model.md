# Proposal: Commentary representation in the runtime event model

Status: draft design note (pre-spec). Satisfies EVE-768 AC#1 (the required
protocol comparison and a chosen design) so an implementation PR can follow with
a correctly-shaped schema instead of the first plausible one. On acceptance this
becomes updates to `specs/events.md` + a Linear implementation issue.

## Problem

Everruns already distinguishes intermediate commentary from the final answer on
**completed** assistant messages via `Message.phase = commentary | final_answer`
(`crates/core/src/message.rs`). The **streaming** event model does not carry that
distinction: `output.message.started` and `output.message.delta`
(`crates/core/src/events.rs`) have neither a message identity nor a phase, so a
consumer cannot tell commentary from a final answer — or even tell two assistant
messages in one turn apart — until `output.message.completed`. Downstream this
collapses five semantically distinct things (commentary, final answer, thinking,
provider reasoning summaries, tool narration) into overlapping UI treatments, and
in particular lets ACP/AG-UI clients render deliberate user-facing progress as
"Thinking" (the EVE-448 class of bug).

## Prior art — how other protocols represent intermediate agent communication

| Protocol | Assistant "commentary" | Final answer | Thinking / reasoning | Commentary↔final distinction | Message identity while streaming |
|---|---|---|---|---|---|
| **AG-UI** (Agent-User Interaction) | `TEXT_MESSAGE_START/CONTENT/END` — ordinary assistant text | same `TEXT_MESSAGE_*` stream | `REASONING_*` (`ReasoningMessageStart/Content`; the older `THINKING_*` names are deprecated) — a **separate** visible-reasoning channel | **None** — both are text; a client infers "not final yet" from message boundaries and whether tool calls follow | `messageId` on every text/reasoning event |
| **ACP** (Agent Client Protocol) | `session/update` → `agent_message_chunk` (role assistant) | same `agent_message_chunk` | `agent_thought_chunk` — a **separate** internal-thought channel | **None** — both are `agent_message_chunk`; commentary mis-routed to `agent_thought_chunk` is exactly why clients label progress "Thinking" | chunk stream is session-scoped; no per-message id in the base message chunk |
| **Codex / OpenAI Responses** (GPT-5.x-codex) | assistant item with `phase: "commentary"` (non-turn-ending: preambles, status) | assistant item with `phase: "final_answer"` (turn-ending) | reasoning summary is a **separate** item; raw CoT is never surfaced | **First-class `phase` field**, and it is load-bearing: dropping it on history reconstruction degrades gpt-5.3-codex | `item/started`, `item/agentMessage/delta`, `item/completed` carry item ids |

Sources: AG-UI events — https://docs.ag-ui.com/concepts/events and https://www.copilotkit.ai/blog/master-the-17-ag-ui-event-types-for-building-agents-the-right-way ; ACP `session/update` — https://agentclientprotocol.com/protocol/v1/overview and https://github.com/agentclientprotocol/agent-client-protocol ; Codex message phases + app-server stream — https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md and the GPT-5 Codex prompting guide https://developers.openai.com/cookbook/examples/gpt-5/codex_prompting_guide .

### What the comparison tells us

1. **Commentary is assistant *text* everywhere, not reasoning.** AG-UI and ACP both carry commentary on the assistant-message channel (`TEXT_MESSAGE` / `agent_message_chunk`) and keep reasoning/thinking on a **distinct** channel (`REASONING_*` / `agent_thought_chunk`). The reported bug ("labeled as Thinking") is a **projection routing** error — commentary sent on the reasoning channel — not a missing streamed phase. Fixing the routing is the load-bearing change.
2. **Only Codex carries a native commentary/final `phase`.** Everruns already adopted exactly this at the completed-message level. AG-UI/ACP have no equivalent flag, so any streamed phase we add is a *hint that degrades to nothing* for those transports — useful, but never a substitute for correct channel routing.
3. **Message identity, not phase, is the missing primitive for streaming.** The concrete "can't tell commentary from final while streaming" and "can't tell two messages in one turn apart" problems (EVE-448) are solved by a stable `message_id` on the streaming lifecycle — which AG-UI (`messageId`) and Codex (item ids) both have and Everruns lacks. Phase is a refinement layered on top.
4. **Phase is provably unknowable at `output.message.started`.** In `crates/core/src/atoms/reason.rs` the started event is emitted *before* the LLM call (≈line 1701/1742); tool calls and provider-native phase only arrive in the terminal `Done` metadata (≈2574/2578); phase is computed only at completion (≈3001–3009). So a streamed `phase` is `None` at start and generally `None` across deltas unless we buffer/reclassify. The completed `Message.phase` must stay authoritative.

## Non-overlapping definitions (EVE-768 AC#2)

- **Commentary** — deliberate, user-visible assistant **text** produced as progress/preamble before or between tool calls; does not end the turn. Durable transcript content. `ExecutionPhase::Commentary`.
- **Final answer** — the durable, turn-ending assistant **text**. `ExecutionPhase::FinalAnswer`.
- **Thinking** — model reasoning **explicitly exposed** by the provider as displayable reasoning (AG-UI `REASONING_*`, ACP `agent_thought_chunk`). Never inferred; raw/hidden chain-of-thought is never persisted or exposed.
- **Reasoning item / summary** — the opaque provider continuation artifact (persisted per EVE-485) plus the safe provider-authored **summary**. The summary is a *reasoning artifact*, surfaced on the reasoning channel by default, not commentary. Policy may allow rendering it as visible detail, but it is never relabeled as an assistant answer.
- **Tool narration / progress / output** — activity scoped to a specific `tool_call` id (its own event families), never merged into the assistant-message channel.

The rule that removes the overlap: **commentary and final answer are the assistant-message channel; thinking and reasoning summaries are the reasoning channel; tool activity is the tool channel — and a projection must never move an item across channels to fake a phase.**

## Recommended design

Adopt the issue's candidate direction (extend the output-message lifecycle; do **not** add a separate commentary event), with message identity as the primary addition and phase as a best-effort refinement:

1. **Add `message_id` to the streaming lifecycle.** `OutputMessageStartedData`, `OutputMessageDeltaData`, and `OutputMessageCompletedData` all carry the same `message_id` (generated up-front in `reason.rs`, reused by `finalize_partial_stream` and `partial_stream.rs` recovery). This is the load-bearing primitive: it lets consumers group deltas, separate multiple messages in one turn, and reconstruct boundaries on replay.
2. **Add `phase: Option<ExecutionPhase>` to started/delta as a hint.** Populate it as soon as it is known: immediately for providers whose stream carries native phase (Codex), otherwise `None` until completion. Document explicitly that `None` means "not yet classified — treat as assistant text," never "thinking." `Message.phase` on `completed` stays authoritative; a consumer that needs certainty waits for `completed` (unchanged from today) but can now render optimistically as text meanwhile.
3. **Phase may change once, monotonically, within a message:** `None → commentary` or `None → final_answer`, never `commentary ↔ final_answer` flip-flop and never *back* to `None`. A provider that reveals native phase mid-stream emits the refined value on subsequent deltas/at completion. Consumers treat a later authoritative value as the correction.
4. **Do not derive phase from "a tool call appeared later."** Deriving commentary purely from the subsequent presence of tool calls is the EVE-448 anti-pattern; classification comes from provider-native phase or, at completion, `from_has_tool_calls` — but the streamed hint never forces a consumer to reclassify already-shown text as reasoning.

### Projection mappings + fallback (EVE-768 AC#6)

| Everruns concept | AG-UI | ACP | Terminal (`examples/coding-cli`) |
|---|---|---|---|
| Commentary | `TEXT_MESSAGE_*` (assistant), do **not** end the stream on it (EVE-448) | `agent_message_chunk` (**never** `agent_thought_chunk`) | inline progress line |
| Final answer | `TEXT_MESSAGE_*` | `agent_message_chunk` | the answer block |
| Thinking | `REASONING_*` | `agent_thought_chunk` | dimmed/collapsible |
| Reasoning summary | `REASONING_*` (or omitted per policy) | `agent_thought_chunk` | collapsible detail |
| Tool narration/progress/output | `TOOL_CALL_*` | `tool_call` / `tool_call_update` | tool activity line |

**Graceful degradation:** when a downstream protocol has no phase concept (AG-UI, ACP today), commentary and final answer both render on the assistant-text channel and are simply shown in order — the transcript stays correct, only the "this was a preamble" affordance is lost. The one hard rule that fixes the reported bug: **a missing/unknown phase must fall back to assistant text, never to the thinking/reasoning channel.**

### Durable replay (EVE-768 AC#4)

Replay reconstructs boundaries from `message_id`-scoped `started`/`delta`/`completed` in `partial_stream.rs` and `PartialStream` (`crates/core/src/traits.rs`), and classification from the authoritative `completed` `Message.phase`. Because started/delta phase is only a hint, replay never depends on it; a stream that died before `completed` replays as unclassified assistant text (the safe fallback), consistent with `finalize_partial_stream`.

### Privacy (EVE-768 AC#5)

Only provider-authored, displayable reasoning (AG-UI `REASONING_*` / ACP `agent_thought_chunk`) and safe reasoning **summaries** are exposed; raw/hidden chain-of-thought is never persisted or surfaced, and commentary — which *is* deliberately user-facing — is never conflated with either. No new field carries raw CoT.

## Why not a separate `commentary` event

A dedicated event would duplicate the assistant-message lifecycle, force every projection to special-case a second text channel, and diverge from AG-UI/ACP/Codex — all of which keep commentary on the assistant-message channel and distinguish it (where they distinguish it at all) by an attribute, not a separate stream. Message identity + an optional phase attribute expresses the required lifecycle cleanly.

## Implementation plan (follow-up issue)

Ordered so each step is independently reviewable:

1. `message_id` on `OutputMessage{Started,Delta,Completed}Data` + emission in `reason.rs`/`finalize_partial_stream` + `partial_stream.rs` recovery; update snapshots (`crates/core/src/snapshots/…output_message_*.snap`), `specs/events.md`, and regenerated `openapi.json`.
2. `phase: Option<ExecutionPhase>` on started/delta as the documented hint; populate from native provider phase where available.
3. Projection updates: AG-UI (`crates/server/src/api/ag_ui.rs`), terminal (`examples/coding-cli`), and the ACP mapping table above (documented now; there is no ACP adapter in-tree yet, so this is a spec-level contract for when one lands).
4. Tests: intermediate commentary around tool calls, reasoning summary vs commentary channel separation, final answer, and replay reconstruction — matching EVE-768 AC#7.

## Open decisions for the maintainer

- Whether to ship `message_id` (step 1) alone first — it is the highest-value, lowest-risk slice and unblocks EVE-448-style consumers immediately — with the streamed `phase` hint (step 2) as a fast follow.
- Whether reasoning summaries default to visible (AG-UI `REASONING_*`) or are policy-gated per harness/agent.
