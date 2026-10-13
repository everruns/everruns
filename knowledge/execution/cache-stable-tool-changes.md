---
type: Decision
title: "Cache-Stable Tool Changes"
description: "Proposed design: the tool list an agent sends stays fixed for a session, and every later change (tool search reveals, MCP servers attached or refreshed, capabilities switched on, client tools) is written into the conversation as a recorded tool change that drivers send in the provider's cache-safe form."
tags:
  - everruns
  - execution
  - caching
  - mcp
---

# Cache-Stable Tool Changes

> Status: **Accepted 2026-10-12. Steps 1 to 3 implemented.**
> Builds on the earlier analysis of Pi's approach (project notes,
> `pi-1-0/cache-stable-tool-reveals.md`) and the caching comparison with
> Pydantic AI (`caching/pydantic-unified-caching.md`), where this is ranked as
> our largest known cause of prompt-cache misses that we cause ourselves.

## Summary

Every provider caches a prompt as an exact prefix, and the tool list comes
first (Anthropic renders tools, then system, then messages; OpenAI renders tool
definitions into the head of the prompt). Changing one byte of one tool throws
the whole cache away: the next request pays the cache-write rate on the entire
conversation instead of the cache-read rate, about 12 times the input cost for
that request on Claude, and a slower first token.

Today our tool list changes in the middle of sessions for many reasons: the
generic `tool_search` rewrites a stub into the full schema, a deferred MCP
server is revealed, ARD attaches a server, an MCP `tools/list` fetch returns
something different or fails, a capability is switched on, an AG-UI client
sends a new tool set, a delegated child finishes. Each one is a full miss.

The fix, which is what Pi, Codex and Claude Code converged on:

1. **The tool list sent in `tools` is fixed for the session.** We call it the
   *baseline*. It is chosen on the first request and recorded.
2. **Every later change is a recorded tool change in the conversation**, a
   typed entry saying which tools were added, redefined or removed (and any
   prompt text that comes with them). It is persisted with the transcript, so
   it survives worker changes, restarts, forks and resumes.
3. **Drivers send recorded changes in the provider's cache-safe form**:
   Anthropic `tool_addition` / `tool_removal` blocks in a mid-conversation
   system message, OpenAI `additional_tools` input items. Where a provider has
   no such form, the driver folds the changes back into `tools` exactly as
   today, so nothing regresses.
4. **One place computes changes.** Before each request, core compares the
   tools the agent can run now with the tools the conversation has declared so
   far and records the difference. Tool search, MCP, ARD, capabilities and
   client tools do not each need their own handling.

## Measurements

### Anthropic (2026-10-06, from the earlier analysis)

Prefix of about 6.4k tokens, two requests per scenario, Sonnet 5.5 and Opus 5.5.

| Scenario | Request 2 cache read |
|---|---|
| No change | 6402 / 6404 |
| **Today:** rewrite the stub to the full schema in `tools` | **0 / 0** |
| New tool through `tool_addition` (beta `inline-tools-2026-09-15`) | 6339 / 6339 |
| Redefine the existing stub by name through `tool_addition` | 6499 / 6496 |

The model called the tool correctly in every `tool_addition` case.

### Anthropic through the driver (2026-10-12)

`test_anthropic_tool_reveal_keeps_prompt_cache` in
`crates/llm-tests/tests/tool_search_test.rs`: generic `tool_search` reveals
`add` mid-turn, and the call after the reveal must read the cached prefix.
With inline changes, that call read the whole prefix (8140 of about 8.5k
tokens, 40480 with a longer prompt) on Opus 5.5, Opus 5, Opus 4.8, Sonnet 5.5,
Haiku 5.5 and Fable 5.1, and every model called `add`. With the flag off,
Haiku 5.5 read 0 and wrote the prefix again. Sonnet 5 read 0 in one of two
runs and Haiku 4.5 in both, so neither has the flag. Fable 5 answered without
searching in two runs, so it stays unverified.

### Anthropic hosted tool search (2026-10-12)

Raw API probe, prefix of about 15k tokens, hosted `tool_search_tool_bm25`, 12
deferred tools plus one non-deferred. Request 2 adds `deploy_service` and asks
for it; the model searched and called it correctly in every case. Each request
runs two model passes (search, then the call), so the read is reported twice.

| Change on request 2 | Opus 5.5 | Sonnet 5.5 | Haiku 5.5 |
|---|---|---|---|
| Append a deferred tool to `tools` | 30338 read, 0 written | same | same |
| Insert a deferred tool mid-list | 30338 read, 0 written | n/a | n/a |
| Remove a deferred tool | 30338 read, 0 written | same | same |
| Append a non-deferred tool | 15250 read, **15250 written** | n/a | n/a |
| `tool_addition` (deferred) after the user turn | 30338 read, 0 written | same | same |

Deferred tools are outside the cached prefix, so with hosted search the
fallback (send the tools wanted now) is already cache-safe for every
deferrable tool. Only a change to a non-deferrable tool rewrites the prefix;
those are a small fixed set, so hosted search keeps the fallback. A
`role: "system"` message must follow a user turn or a server tool result.

### OpenAI Responses through the driver (2026-10-12)

`test_openai_tool_reveal_keeps_prompt_cache`, the same scenario as the Anthropic
driver test. The call after the reveal read 4909 tokens (the whole prefix
written by the first call) on GPT-6 Astra, Sol, Luna and 6.1 Sol, and each
model called `add`. With the flag off, Luna read 0 and wrote 5075. GPT-5.6
Terra passed the same test (step 5).

### OpenAI Responses (2026-10-12)

Prefix of about 5.6k tokens, `cached_tokens` of request 2. Every case where the
tool was available produced the correct call `deploy_service({"env":"prod"})`.

| Scenario | gpt-5.5 | gpt-6.1-sol | gpt-6-luna |
|---|---|---|---|
| No change | n/a | 5564 | n/a |
| Append a new tool to `tools` | n/a | **0** | n/a |
| New tool through an `additional_tools` developer item | n/a | 5564 | n/a |
| Redefine an existing stub by name through `additional_tools` | **0** | 5588 | 5588 |
| Hosted `tool_search`: append a deferred tool to `tools` | **0** | **0** | **0** |
| Hosted `tool_search`: add a deferred tool through `additional_tools` | 5632 | 5924 | 5924 |

A second probe (step 5) put the same two cases to the other gpt-5.x models:

| Scenario | gpt-5.4 | gpt-5.5 | gpt-5.6 Sol, Terra, Luna |
|---|---|---|---|
| New tool through `additional_tools` | 4608 | 4608 | 5564 |
| Append a new tool to `tools` | **0** | **0** | **0** |
| Redefine a stub through `additional_tools` | **0** | **0** | 5588 |

So gpt-5.6 gets the full mode (`inline_tool_changes`), and gpt-5.5 and gpt-5.4
the add-only mode (`inline_tool_additions`): an epoch whose changes include a
redefinition falls back to sending the wanted tools.

Two things follow. On GPT-6 models a stub can be redefined in place, the same
as on Claude. On gpt-5.5 it cannot, so there the stub must never be in `tools`
in the first place (see [OpenAI](#openai-responses)). And hosted tool search
does not make MCP tools that arrive later cheap: they still have to arrive
through `additional_tools`, not `tools`.

## State of the art

| Who | What they do |
|---|---|
| Anthropic API | Hosted tool search keeps deferred tools out of the cached prefix and expands them inline. Mid-conversation `role: "system"` messages (no header). `tool_addition` / `tool_removal` by reference (beta `mid-conversation-tool-changes-2026-07-01`, also Bedrock and Google Cloud) and by full definition (beta `inline-tools-2026-09-15`, Claude API only, same-name redefinition allowed). Keep one non-deferred tool in `tools`. Cache diagnostics (GA 2026-09-23) return `cache_miss_reason: tools_changed`. The MCP connector can pin a server's tool list for a conversation. |
| OpenAI Responses | Hosted and client-executed `tool_search`; discovered tools are injected at the end of the context. `additional_tools` developer input items add tools mid-conversation. `tool_choice: allowed_tools` restricts calls without touching the list. Chat Completions has only a fixed list plus `allowed_tools`. |
| Gemini | Explicit `cachedContent` freezes tools; no mid-conversation tool mechanism. |
| Pi | `declareToolChanges()` diffs runnable tools against tools declared in the transcript before every request and records `toolsAdded` / `toolsRemoved`. Anthropic: `tool_addition` by value; OpenAI: `additional_tools` or a synthetic client `tool_search` pair; Kimi: a system message carrying `tools`. Falls back to resending the full list (one miss) when a transport cannot express the change. |
| Codex CLI | For some models sends no `tools` at all: the whole catalogue goes in a leading `additional_tools` item, then hashed diffs add new or changed tools as `additional_tools` and announce removals in a developer notice. |
| Claude Code | MCP tools deferred by default; keeps the first request's tool list for the whole conversation; mode changes (plan mode) are tools plus system messages, never tool swaps. A server that disconnects keeps its definitions; calls error. |
| Manus | "Mask, don't remove": definitions never change; the allowed set is constrained by decoding. |
| Pydantic AI, Vercel AI SDK | By-reference `tool_addition` on Anthropic, `additional_tools` on OpenAI (Pydantic). The AI SDK's `activeTools` filters the request and breaks the cache. |
| MCP spec | `notifications/tools/list_changed` makes the client refetch `tools/list`; nothing about caching or order. |

The shared lesson: never edit the top-level list; append changes to the
conversation; keep a fallback that resends the full list and accepts one miss.

## Where our tool list changes today

Context is rebuilt on every reasoning step (`crates/core/src/runtime_context.rs`,
`build_runtime_agent`), so anything below shows up on the next step.

| Source | Where | State today |
|---|---|---|
| Generic `tool_search` reveal rewrites the stub | `crates/core/src/builtins/tool_search.rs` (`DeferSchemaHook`) | the conversation: `tool_search` results and recorded stubs (was process memory until step 5) |
| Deferral threshold: when the tool count crosses 15, every deferrable tool flips between stub and full | same file, `transform` | recomputed each step |
| Deferred MCP server revealed: the placeholder disappears and the server's tools are inserted mid-list | `crates/contracts/src/runtime/mcp_deferred.rs`, `crates/worker/src/reveal_storage.rs` | session record `mcp_reveal:*` |
| ARD or chat-scoped MCP attach; A2A targets change `spawn_agent` and its prompt | `crates/core/src/ard_attachment.rs` | session records |
| MCP `tools/list`: scoped servers without a preset are fetched live on every turn load; a failed fetch silently drops the server's tools; order is whatever the server returns | `crates/server/src/domains/mcp_servers/scoped_mcp/`, `crates/core/src/host/mcp_cache.rs` | live, or caches with TTL |
| Host tools that come and go (`report_result` with child tasks, coordinator and worker tools, MCP resource tools) | `crates/capabilities/src/host_extension.rs`, `crates/core/src/host/host/reason_surface.rs` | recomputed each step |
| AG-UI frontend tools replaced per run (order-sensitive compare) | `crates/server/src/channels/ag_ui/mod.rs` | session record |
| Capability switched on or off in a session (yolop `/ext`, in-process overlay), agent or harness edits, a model override that changes `auto_tool_search` resolution | `crates/core/src/host/runtime.rs` | session and agent records |
| `tools_in_shell` and Lua code mode rewrite the `bash` / `lua` description with the hidden-tool catalogue | `crates/integrations/src/bashkit/tools_in_shell/`, `lua/code_mode.rs` | recomputed each step |
| Hosted Claude search switches on when the count crosses the threshold, which reshapes `tools` | `crates/drivers/drivers/src/anthropic/driver.rs` | recomputed each step |

Not affected: native hosted search once a session's tool set is fixed, because
the provider expands tools inside the conversation.

The same pattern shows up in the system prompt (skills list, soft-approval
mode, A2A targets, yolop's memory how-to after a reveal). Those are covered by
[step 6](#plan) with the same mechanism, because a capability switched on
mid-session usually brings both tools and prompt text.

## Design

### 1. Baseline

The first request of a session fixes the baseline: the tool list as it is
then, after every definition hook (so deferred tools are stubs), plus the
decisions that shape it: whether deferral is on, and which hosted search mode
applies. The baseline is recorded on the first tool-change entry (below), so it
is in the transcript like everything else.

Decisions that today are re-made every step become sticky for the session:
the deferral threshold is checked once, and `auto_tool_search` resolution is
fixed at the baseline. A model switch mid-session (which already restarts the
cache) re-baselines.

Within the baseline, order is deterministic: capability order as today, MCP
tools sorted by name within each server.

### 2. Recorded tool changes

One recorded entry, `ToolChanges` (`crates/contracts/src/tool_changes.rs`),
holding

- `baseline`: present on the first entry of an epoch, the full list sent as
  `tools`,
- `added`: full definitions of tools that become callable,
- `redefined`: full definitions that replace an earlier definition of the
  same name (a stub becoming its full schema is a redefinition),
- `removed`: names that are no longer callable,
- `note`: optional prompt text that comes with the change (a capability's
  how-to, "these tools are no longer available").

Entries are stored on the metadata of the completed assistant message whose
request they preceded, following the precedent of Astra effort snapshots
(`configuration_update`, rendered natively and never shown as conversation
text). That keeps them in the immutable transcript without a new message kind
in chat, and the reason step replays them to the input they took effect at
(`crates/core/src/engine/execution/reason/tool_changes.rs`). The driver
`Message` carries the entry as `tool_changes` on that input. Replaying baseline
plus entries in order always yields the tool set the conversation has seen.
That makes the conversation, not process memory, the record of what was
revealed, which also lets `tool_search` drop its in-memory registry (step 5)
and fixes reveals being forgotten after a restart or on another worker.

An epoch ends where the cache restarts anyway: an answer from another model,
or a transcript cut by a compaction checkpoint (the earlier answers, and so
the baseline, are no longer in the loaded messages). The next request records
a new baseline.

### 3. One comparison before each request

In the reason step, after the tool list is built for this step:

1. Replay baseline plus recorded changes into the *declared* set.
2. Compare it with the *wanted* set: what the agent can run now, with every
   hook applied.
3. If they differ, append one `tool_changes` entry with the difference. Batch
   everything that changed in one step into one entry.
4. Send the request with `tools` = baseline, never the wanted set.

Sources stop editing tool definitions in place and instead change what is
wanted:

- **Generic `tool_search`** stops rewriting stubs. A reveal makes the full
  definition wanted; the comparison records it as a redefinition. The tool
  result still returns the schema for the model to read.
- **Deferred MCP servers, ARD, capability switches, AG-UI client tools, host
  tools, MCP refetches** all just change the wanted set.
- **Removal is soft.** A removed tool stays declared, is announced as removed,
  and a call to it is refused with a clear error. This matches "mask, don't
  remove": nothing in the prefix changes, and a model that calls it anyway
  learns why.
- **A failed MCP fetch is not a removal.** The last declared definitions stay;
  a call returns "server unavailable". Today the tools vanish for that load,
  which is both a cache miss and a confusing tool disappearance.
- **MCP `notifications/tools/list_changed`**, when we support it, feeds the
  same comparison and needs nothing of its own.

### 4. Drivers

Each driver decides per model, from model profile parameters set only after a
live round trip, matching our rule for `tool_search`. Anthropic uses
`inline_tool_changes` (`crates/contracts/src/model_profile_data/types.rs`);
OpenAI adds its own in step 4, since it can add but not always redefine.

The reason step hands drivers the epoch's baseline (`LlmCallConfig::tool_baseline`)
only when every change since it rides on a message of the request. If a model
view or trimming dropped one, the request carries no baseline and every driver
sends the tools wanted now, as before.

#### Anthropic

Implemented in `crates/drivers/drivers/src/anthropic/driver_tool_changes.rs`,
on the families a live probe verified (the profile flag lists them), against
the first-party endpoint only:

- `tools` is the baseline followed by a deferred placeholder tool, as Pi does.
  Pi measured a full miss on the first change without it; our probe saw no
  difference, and it costs nothing.
- Each `tool_changes` entry becomes a mid-conversation system message before
  the input it took effect at: `tool_addition` blocks carrying the full
  definition (beta `inline-tools-2026-09-15`) for added and redefined tools,
  and one text line for `note` and removals. The layout code places it after
  that input and before the next assistant turn (`driver_layout.rs`).
- Removal is soft: announced in that text line, not `tool_removal`.
- Only one system message fits a position, so when the same turn also has
  turn-scoped reminders (`<facts>`), they join the change's message and lose
  `clear_at` for that turn. A change has no other place to go, and the replay
  lays it out the same way every time.
- Fallback to the wanted tools, per request: hosted Claude search, the native
  computer and browser toolsets, an empty baseline, a gateway endpoint, or a
  change with no user turn before it (it would land in top-level `system`,
  which cannot hold a definition).
- Hosted Claude search keeps the fallback: deferred tools are outside the
  cached prefix, so sending the current tools is already cache-safe for them
  (measured above).
- Bedrock and Google Cloud have only the by-reference form; a reference must
  name a tool already in `tools`, so on those hosts only removals and
  re-offers of baseline tools are cache-safe. Requests there fall back until
  by-value support reaches them. OpenRouter's Anthropic endpoint likewise
  accepts only the by-reference form (checked 2026-10-12).

#### OpenAI Responses

Implemented in `crates/contracts/src/openresponses_protocol/tool_changes.rs`,
on models with `inline_tool_changes` (the GPT-6 family), against
`api.openai.com` only (the OpenAI driver opts in; compatible endpoints never do).

- Additions are `additional_tools` developer items at the entry's position:
  after the input the change took effect at, before the next model output.
- On models where redefinition is cache-safe (GPT-6 family, measured above),
  redefinitions go the same way.
- On models where it is not (gpt-5.5), the generic `tool_search` puts deferred
  tools in the baseline **without** stubs: their names and one-line
  descriptions go in the `tool_search` tool's own description, which is fixed
  at the baseline, and a reveal is an addition, not a redefinition.
- With hosted search, later tools are `additional_tools` items with
  `defer_loading: true` (measured cache-safe above on all three models).
- Removals are a developer note plus the runtime refusal, as Codex does.

#### Everything else (fallback)

Gemini, OpenAI Completions, OpenRouter, Bedrock Converse and unknown models:
the driver replays the recorded changes into `tools` and sends that, which is
exactly today's behavior, so this design is never worse. Two cheap
improvements on top:

- **Change at cold moments.** When nothing forces it, hold a non-urgent change
  (a removal, a refreshed MCP description) until the cache is cold anyway:
  after compaction, after an idle gap longer than the cache lifetime, at a
  model switch. Reveals and additions the model asked for go out immediately.
- **Optional schema-in-result mode** for the generic `tool_search`: the stub
  stays permissive forever, the model reads the full schema from the search
  result, and arguments are validated at execution. It never changes `tools`,
  but call quality may drop. Experimental, and only switched on per model if a
  Mira eval shows no loss.

### 5. Compaction and trimming

Compaction already rewrites history and costs a miss, so it is the natural
point to fold every recorded change into a new baseline: the compacted
conversation starts with the full current tool list in `tools` and no change
entries. Infinity-context trimming must never drop a `tool_changes` entry; if
the trimmed range contains one, trimming re-baselines too.

A related problem found while surveying, fixed in step 1: the infinity-context
"earlier messages are not in this context" notice and the compaction summary
are both inserted at index 0, right after the agent prompt, where the
Anthropic layout folds them into top-level `system`. Each new notice or summary
therefore rewrites the system prompt.

### 6. Seeing it work

- Record Anthropic's `cache_miss_reason` (GA cache diagnostics) on every
  generation when diagnostics are on, and show it on the Session Trace cache
  bar. `tools_changed` after this lands is a bug.
- A provider-free test in CI: a scripted session (reveal, deferred MCP reveal,
  ARD attach, capability switch, client tools, child task finishing) run
  through each driver's request builder, asserting that `tools` and `system`
  are byte-identical across requests on models with `tool_changes` support,
  and that the fallback produces exactly today's requests elsewhere.
- Live `llm-tests` cases per provider asserting the cached share stays above
  90% across a reveal and an MCP attach, which is the gate for setting the
  profile flag.

## Plan

Each step is one PR and leaves `main` working.

1. **Cheap stability fixes, no new mechanism.** Done: the generic
   `tool_search` keeps deferring once a session has crossed the threshold,
   MCP tools are sorted by name within a server, and AG-UI frontend tools sent
   in a new order keep the stored order. The driver prefix-stability test
   landed separately (`crates/drivers/drivers/tests/prompt_prefix_wire.rs`).
   Two items moved: keeping last-known MCP tools on a failed fetch needs the
   recorded declarations of step 2, so it is in step 5; keeping the
   infinity-context notice and compaction summary out of top-level `system`
   needs them sent as something other than a leading system message, so it is
   in step 6.
2. **Recorded tool changes in contracts and core.** Done: the `ToolChanges`
   entry, the baseline, the comparison in the reason step and persistence on
   assistant messages. Every driver uses the fallback, so requests are
   unchanged. Session Trace display moved to step 3, where changes first reach
   a request.
3. **Anthropic driver** renders changes inline and Session Trace shows them;
   profile flag for verified families; live tests once the key has credit.
   Done: the request side; Session Trace listing each call's change
   (`llm.generation` carries it as `metadata.tool_changes`; a cache reset is
   blamed on tools only when the provider body's tools changed); and the live
   cache test, which widened the flag to Opus 5 and 4.8, Haiku 5.5 and Fable
   5.1 (see the 2026-10-12 driver results above).
4. **OpenAI Responses driver** renders `additional_tools`; profile flag
   (`add_and_redefine` for GPT-6, `add_only` for 5.x) with live tests.
   Done for GPT-6 (add and redefine, live-tested). The add-only mode for
   gpt-5.x moves to step 5, the first step where a source produces additions
   rather than redefinitions.
5. **Sources move onto the comparison**: generic `tool_search` (drop the
   in-memory registry), deferred MCP, ARD, capability switches, AG-UI, host
   tools, `tools_in_shell` catalogue. A failed MCP fetch keeps the last
   declared definitions. Done for `tool_search`: the in-memory registry is
   gone; context assembly reads reveals from stored `tool_search` results and
   keeps deferral on once a recorded tool list holds a stub, so reveals
   survive restarts and look the same on every worker. Done for OpenAI
   profiles: gpt-5.6 Sol, Terra and Luna take the full mode, gpt-5.5 and gpt-5.4
   the add-only mode (probe above). Done for a failed MCP listing: the session
   records what each server listed last (`MCP_LAST_TOOLS_KV_PREFIX`, reserved
   from `kv_store`) and a turn whose `tools/list` fails publishes that record,
   so the tool list does not change and a call fails with the server's
   connection error. The record is session storage rather than the recorded
   declarations, because the turn's tool registry is built before the reason
   step reads the transcript; the in-process runtime already serves stale tools
   on a failed refresh.
6. **Prompt text with changes**: capability prompt additions that appear
   mid-session ride in the change's `note` instead of the system prompt;
   compaction and trimming re-baseline; the infinity-context notice and
   compaction summary stop being folded into top-level `system`.
   Done: a baseline entry also records the system prompt
   (`ToolChanges::system_prompt`); later parts (split on blank lines, the
   `<system-prompt>` wrapper ignored) that a capability adds become the entry's
   `note`, and inline requests send the epoch's prompt
   (`LlmCallConfig::system_prompt_baseline`). A part that changes or
   disappears sends the current prompt, as before. Live
   (`test_prompt_text_added_mid_session_keeps_prompt_cache`, how-to inserted
   ahead of 2-4k tokens of capability text): Sonnet 5.5 read 4405 of 4551,
   GPT-6 Luna 2478 of 2481, and both followed the added text; without the
   prompt baseline they read 714 and 0. Compaction and trimming needed
   nothing: a checkpoint cuts the stored entries, so the next request records
   a new baseline, and a trimmed entry withholds the baseline (fallback). The
   infinity-context notice already sits after the anchored head, so it stays a
   mid-conversation message; only the compaction summary folds into `system`,
   once per compaction, which rewrites the prefix anyway.
7. **yolop** adopts it (its reveal-gated memory how-to moves into the `note`).
8. **Fallback experiments**: cold-moment batching, then the schema-in-result
   eval. Done for batching, removals only: on a model with neither inline
   parameter, a request whose only change is a removal holds it while the
   last answer is younger than the cache lifetime (the 1h retention, else 5
   minutes on Anthropic and Bedrock, else 30; overstating only holds longer).
   The removal is not recorded, the held definitions go back into `tools` in
   their declared places, and a call to one fails like any undeclared tool
   because the act step runs only the agent's current tools. It goes out with
   the next addition, redefinition or note, or after the idle gap; compaction
   and a model switch re-baseline anyway. Redefinitions are not held: a stale
   schema would make calls fail. Live
   (`test_tool_removal_waits_for_a_cold_cache`, OpenRouter GPT-6 Luna): the
   call after a capability is switched off read 2646 of 2649 cached tokens,
   0 without the hold. Only first-party Anthropic and OpenAI count as inline:
   a gateway serving the same model resolves to the same profile but sends the
   full tool list.
   Done for schema-in-result, kept opt-in: `tool_search` config
   `{"reveal": "in_result"}` keeps the stub listed and the model calls from the
   search result's schema; the act step validates every call against the
   registered schema either way. The study (`evals/tool-search-reveal`, 8
   cases x 3 trials per arm, OpenRouter) found no loss on GPT-6 Luna (24/24
   correct, 24/24 first try in both arms) or Mistral Large 4 (24/24 correct in
   both; first try 17/24 redefine, 21/24 in_result), but Muse Spark 1.3 fell
   from 24/24 to 0/24: against a property-less stub it sends every number,
   boolean and object as a JSON string and never recovers. A per-model default
   would need a profile flag; until someone asks, agents opt in.

## Open questions

- **Removal style.** Proposed: soft removal (announced, call refused). The
  alternative is Anthropic's real `tool_removal`, which we can add later since
  it is also cache-safe; OpenAI has no equivalent.
- **Inline tools on Bedrock and Google Cloud.** Today only by-reference changes
  work there, so additions fall back. Revisit when Anthropic extends the beta.

## References

- Anthropic: [tool search](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool),
  [mid-conversation system messages and tool changes](https://platform.claude.com/docs/en/build-with-claude/mid-conversation-system-messages),
  [tool use with prompt caching](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-use-with-prompt-caching),
  [cache diagnostics](https://platform.claude.com/docs/en/build-with-claude/cache-diagnostics),
  [MCP connector](https://platform.claude.com/docs/en/agents-and-tools/mcp-connector)
- OpenAI: [tool search](https://developers.openai.com/api/docs/guides/tools-tool-search),
  [prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching),
  [function calling](https://developers.openai.com/api/docs/guides/function-calling)
- Pi: `packages/agent/src/agent-loop.ts` (`declareToolChanges`) in
  [badlogic/pi-mono](https://github.com/badlogic/pi-mono);
  [Earendil on prompt caching](https://earendil.com/posts/prompt-caching/)
- Codex: `codex-rs/core/src/context/world_state/` in
  [openai/codex](https://github.com/openai/codex)
- Claude Code: [prompt caching](https://code.claude.com/docs/en/prompt-caching),
  [MCP](https://code.claude.com/docs/en/mcp)
- Manus: [Context Engineering for AI Agents](https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus)
- MCP: [tools, list changed](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
- Internal: [Tool Search](tool-search.md), [LLM Drivers](../foundations/llm-drivers.md)
