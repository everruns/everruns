---
type: Specification
title: "Anthropic Infinity Context Compaction"
description: "Append-only Anthropic message history through server-side threshold compaction."
tags:
  - everruns
  - runtime-resources
  - anthropic
---
# Anthropic Infinity Context Compaction

## Summary

Infinity Context currently moves a counted hidden-history notice and a recent
window through Anthropic `messages`. Each move changes an earlier prompt prefix,
which invalidates prompt caching and can invalidate preserved thinking. For
supported direct Anthropic models, Infinity Context will instead keep the prior
wire-level `messages` array append-only and enable Anthropic threshold
compaction. Anthropic will reduce the model-visible context on the server.
Everruns will keep raw session history lossless and queryable.

This specification is the provider-specific exception to the client-side
windowing rules in [Infinity Context](infinity-context.md). It does not change
other providers.

## Product behavior

1. When Infinity Context uses an eligible Anthropic provider and model,
   Everruns MUST use the native mode defined in this specification.
2. Native mode MUST preserve this invariant for consecutive successful requests
   in one session, provider, and model epoch:
   - Let `M(n)` be the JSON value of request `n`'s complete `messages` array.
   - `M(n)` MUST equal the first `len(M(n))` entries of `M(n+1)`.
   - The comparison includes roles, content-block order, native thinking,
     tool-use blocks, compaction blocks, and every field within those blocks.
   - The assistant response to request `n` and the next user or tool turn are
     appended after that prefix.
   - A model/provider change or a separate, deliberate prompt-history rewrite
     starts a new epoch. Infinity Context itself MUST NOT cause such a rewrite.
3. Native mode MUST NOT apply Infinity Context's candidate limit, head/tail
   trimming, or hidden-history notice. It MUST retain `query_history`, the
   Infinity Context system instruction, access-control filters, tool-call
   integrity checks, and unrelated prompt filters.
4. Native mode MUST be the only provider-visible history reducer for the call.
   When the generic `compaction` capability is also active, bypass its local
   observation masking, summarization, trim, and standalone native-compact
   stages. Those stages rewrite earlier input and violate the prefix invariant.
5. Native mode MUST send Anthropic threshold compaction on every request in the
   epoch. When Anthropic returns a compaction block, Everruns MUST append and
   replay the complete native response without changing the block.
6. Raw event history MUST remain unchanged. Conversation APIs, audit, export,
   forks, and `query_history` MUST continue to use raw history rather than the
   lossy compaction summary.
7. An ineligible provider, model, endpoint, or prompt-rewrite configuration
   MUST retain the current provider-neutral Infinity Context window and counted
   notice. The prefix guarantee does not apply to that fallback.
8. A fallback or native-mode failure MUST be observable. Everruns MUST NOT
   report a successful native mode while silently sending a rewritten prefix.
9. Existing agent configuration remains valid. In native mode,
   `context_budget_tokens` controls the server compaction trigger.
   `min_recent_messages`, `max_recent_messages`, and `keep_first_messages`
   remain effective only in legacy fallback mode.

## Technical design

### Current state

- `InfinityContextFilterProvider` applies a bounded candidate load, trims the
  loaded history, and inserts a counted notice
  (`crates/builtins/src/infinity_context.rs:296-366`).
- `ExcludedNoticeTransform` renders the changing count
  (`crates/core/src/message_filter.rs:176-208`).
- Host assembly applies those filters before it resolves the per-turn model
  override (`crates/host/src/runtime_context.rs:242-274`).
- The Anthropic driver places later system messages in `messages` for selected
  models, but its two moving message-level cache markers still edit prior
  blocks (`crates/drivers/anthropic/src/driver_layout.rs:160-207`).
- The Anthropic stream parser already retains complete provider-native response
  content as internal `provider_opaque_content`
  (`crates/drivers/anthropic/src/driver.rs:1330-1358`).
- Durable compaction checkpoints already store encrypted provider/model-specific
  state beside the immutable event log
  (`crates/core/src/compaction_checkpoint.rs:11-61`,
  `crates/server/src/storage/compaction_checkpoint_store.rs:10-108`).

A byte-stable notice would fix only the notice entry. The recent window would
still move, so the full prior prefix would still change.

### Native-mode eligibility

Native mode requires all of these conditions:

- The `infinity_context` capability is active.
- The resolved driver is Anthropic Messages against Anthropic's first-party API.
  Custom base URLs, compatible gateways, Bedrock, and other protocol adapters
  use legacy fallback until they declare and test equivalent support.
- The resolved `ModelProfile` has `supports_server_compaction: true`. Add this
  field with a serde default of `false`; do not infer support from context size,
  reasoning, adaptive thinking, or model-name ordering.
- The normalized model family is one Anthropic documents for threshold
  compaction and Everruns currently profiles:
  - `claude-fable-5-1`
  - `claude-fable-5`
  - `claude-opus-5`
  - `claude-opus-4-8`
  - `claude-opus-4-7`
  - `claude-opus-4-6`
  - `claude-sonnet-5`
  - `claude-sonnet-4-6`
- Dated aliases and `[1m]` variants inherit the normalized family's explicit
  profile value. `claude-opus-5-5`, Haiku, and an unknown future family remain
  false until Anthropic documents support and a profile test enables it.
- No active `user_prompt_submit` hook or other configuration requires rebuilding
  provider-visible history from a different raw audit representation. This
  fails closed to legacy mode so filtered content cannot be resent.

The model-profile capability is the rollout and rollback control. A family can
be disabled independently without changing stored sessions.

### Model-aware history loading

Turn assembly needs two stages because the selected model can be overridden by a
recent control message:

1. Load only the bounded recent control-bearing history needed to resolve the
   latest model override. Do not inject the Infinity notice in this pass.
2. Resolve the provider, endpoint, model profile, and native-mode eligibility.
3. Rebuild the final message query with a derived
   `anthropic_server_compaction_active` flag in the Infinity Context filter
   config.
4. In native mode, that flag suppresses only Infinity Context's candidate
   limit, anchor load, post-load trim, and notice. It also suppresses the
   generic compaction capability's provider-visible reducers for this call.
   Apply every other applicable filter.
5. In fallback mode, run the existing query and post-load behavior unchanged.

The first pass MUST NOT become the model input. The second pass is authoritative.

### Anthropic request

For native mode, the Anthropic driver MUST add:

- Beta header `compact-2026-01-12`.
- `context_management.edits` with one `compact_20260112` entry.
- `pause_after_compaction: false`.
- No custom compaction instructions.
- An input-token trigger:
  - Start with Infinity Context's `context_budget_tokens`.
  - Raise values below Anthropic's minimum to `50_000`.
  - Cap the value at `effective_context_window - max_tokens`.
  - If that cap is below `50_000`, mark the model ineligible instead of sending
    an invalid request.

Do not use on-demand/background `compact-2026-09-04`. That mode requires a
client-side prefix replacement.

The streaming parser MUST:

- Preserve every native response content block in order.
- Recognize `compaction` block starts and `compaction_delta` events.
- Accumulate readable `content` fragments in order.
- Preserve `encrypted_content` verbatim.
- Reject an incomplete or structurally invalid compaction block before treating
  the stream as successfully complete.
- Keep the block in internal provider-owned content. Public message projections,
  logs, metrics, and lifecycle events MUST NOT expose its content.

### Prompt caching

Native mode MUST replace moving message-level cache breakpoints with Anthropic
automatic caching:

- Add top-level `cache_control: {"type":"ephemeral"}` when prompt caching is
  enabled.
- Do not call `mark_recent_text_blocks_for_cache` in native mode.
- Retain the existing stable explicit breakpoint on the system prompt and the
  existing stable explicit breakpoint on the tool array. Together with the
  automatic breakpoint, this uses at most three of Anthropic's four slots.
- Continue recording disjoint `cache_read_input_tokens` and
  `cache_creation_input_tokens`.

The top-level field lets Anthropic move its logical cache breakpoint without
inserting or moving fields inside `messages`. Changes to the model, tools,
system prompt, thinking controls, images, or other request options can still
invalidate Anthropic's broader prompt cache; this work removes Infinity
Context's history rewrite.

### Replay and durable checkpoints

Raw events are the correctness fallback. An encrypted checkpoint is a load-time
optimization after Anthropic has produced a compaction block.

1. The driver forms a provider-owned checkpoint candidate from the exact
   serialized request `messages` followed by the complete native assistant
   response. Carry it internally on completion metadata. Do not expose it on
   public events.
2. Persist the assistant `output.message.completed` event first, including the
   existing internal provider-native content. Capture the event sequence
   returned by durable emission.
3. If the response contains a valid compaction block, install a monotonic
   checkpoint at that event sequence.
4. Add an opaque provider context variant equivalent to:
   `AnthropicMessagesPrefix { messages_json: String }`

   `messages_json` contains the exact serialized `messages` array after the
   compaction response is appended. The neutral runtime MUST NOT parse,
   normalize, reorder, prune, or translate it. The Anthropic driver validates
   that it is an array and appends the converted suffix without reserializing
   stored entries.
5. On a later matching turn, load raw events strictly after the checkpoint
   sequence, convert only that suffix, and append it to the exact stored prefix.
6. Without a usable checkpoint, load all filtered raw history and reconstruct
   the same native blocks. A crash or checkpoint write failure after output
   completion therefore affects performance, not the transcript contract.
7. A checkpoint is compatible only with the same provider type, exact resolved
   model, and Anthropic prefix format. A model/provider switch ignores it.
8. Use Anthropic prefix format version 2 while OpenAI checkpoints remain version
   1. Older binaries ignore version 2 and rebuild from raw history. No database
   migration is required because format version is already part of the unique
   key.
9. Keep the existing encrypted-at-rest storage and 32 MiB plaintext payload
   limit. An oversized candidate is not installed. The runtime records the
   condition and uses raw replay.

This checkpoint differs from OpenAI native compaction. OpenAI stores a semantic
replacement. Anthropic stores the complete client wire prefix while Anthropic
ignores content before the latest compaction block on the server.

### Data handling

- Native mode sends Anthropic the complete provider-visible transcript on every
  request, including messages that legacy Infinity Context would have kept only
  in Everruns storage. This added provider egress is the intentional cost of
  full-prefix preservation.
- Prompt-rewriting hooks remain ineligible until Everruns has a durable
  provider-visible history distinct from raw audit events. Raw replay MUST NOT
  bypass a hook that removed sensitive content.
- The checkpoint contains the same sensitive transcript plus provider-native
  compaction state. It MUST use the existing encryption service, organization
  boundary, and public-projection redaction.
- Anthropic's beta data-retention eligibility remains an operator/provider
  contract. Everruns MUST NOT claim that server compaction changes the
  configured provider's retention policy.

### Failure and fallback

- Proactively ineligible requests use the legacy window before network I/O.
- If Anthropic rejects the compaction beta or edit on the first native attempt
  before any stream event, retry once with the legacy window. Record a bounded,
  process-local negative capability entry keyed by endpoint and normalized
  model family. Cap the map at 4,096 entries with oldest-entry eviction, matching
  the existing proactive-compaction retry tracker, so later turns do not probe
  again until process restart or eviction.
- After a native request succeeds in an epoch, do not answer a later
  `RequestTooLarge` or beta error by rewriting that epoch's prefix. Return the
  classified provider error and emit the native failure state.
- Do not retry after a compaction block, text delta, tool call, or other
  externally visible stream output. Existing stream replay safety rules remain
  authoritative.
- If checkpoint installation fails after the raw completed output is durable,
  keep the successful model turn and rebuild from raw history next time.
- If native response content cannot be persisted durably, fail the turn. Never
  persist only the public text and discard a compaction block required for the
  next request.
- A corrupt or incompatible checkpoint is never sent. Ignore it, emit a
  diagnostic, and rebuild from raw events.

### Observability and rollout

Use the existing compaction lifecycle:

- Emit `context.compacting` when the stream first opens a compaction block.
- Emit `context.compacted` only after the complete native response is durable.
  Include provider, model, trigger, duration, input/output usage, checkpoint ID
  when installed, checkpoint bytes, and replay source (`checkpoint` or `raw`).
- Emit `context.compaction.failed` for an invalid block, lost native content, or
  a terminal native-mode failure. A failed optional checkpoint optimization is
  a warning metric, not a failed model compaction.
- Add generation metadata for selected reduction mode, compaction-block
  observation, trigger, fallback reason, and replay source.
- Count native selections, legacy fallbacks by reason, beta rejections,
  compaction blocks, checkpoint installs/failures/oversize candidates, raw
  rebuilds, and prefix assertion failures. Never put prompt or compaction
  content in a label or log field.

Enable families through `supports_server_compaction` in small production
cohorts. Compare cache-read ratio, cache-creation tokens, provider errors,
request bytes, raw rebuilds, and checkpoint size before enabling the next
family. Roll back a family by setting its profile capability to false.

## Decisions

### Use threshold server-side compaction

- **Selected:** `compact-2026-01-12` keeps the client transcript append-only and
  lets Anthropic reduce context internally. It satisfies the prefix invariant
  and preserves signed thinking.
- **Rejected:** append another counted notice on every slide. This leaves stale
  or duplicate notices and still changes which recent messages are sent.
- **Rejected:** use one count-free stable notice. This stabilizes one entry but
  the sliding recent window still rewrites the prefix.
- **Rejected:** tool-result or thinking context editing. Those strategies cannot
  summarize arbitrary old user and assistant turns.
- **Rejected:** on-demand/background `compact-2026-09-04`. Its required client
  swap deliberately replaces an earlier prefix.

### Preserve the full client prefix

- **Selected:** continue sending the complete prefix and checkpoint it only as
  an encrypted replay optimization. This matches the requested cache and
  preserved-thinking contract.
- **Rejected:** discard client messages before the latest compaction block.
  Anthropic permits that optimization, but it would fail byte-prefix
  continuity.
- **Trade-off:** request bytes and encrypted checkpoint bytes continue to grow
  even though Anthropic's active semantic context is compact. The 32 MiB
  checkpoint cap and raw replay prevent storage corruption, but this design
  does not promise an unbounded HTTP body.

### Keep provider-neutral fallback

- **Selected:** unsupported configurations keep today's windowing behavior.
  This avoids sending an untested beta to gateways and undocumented models.
- **Rejected:** infer support from a large context window or newer model name.
  Anthropic does not document Opus 5.5 even though it is newer than supported
  families.
- **Trade-off:** Infinity Context has provider/model-specific live-prompt
  semantics. The selected mode and fallback reason must therefore be visible.

## Assumptions

- The requester chose full prior-`messages` prefix preservation, not only a
  stable hidden-history notice.
- Anthropic's documented compatibility list and beta contract as of
  2026-09-24 are authoritative. New families remain disabled until verified.
- Stable agent instructions and tools remain unchanged between compared
  requests. Separate deliberate edits start a new prefix epoch.
- A completed output event exposes its durable sequence to the checkpoint
  installer. If an adapter does not expose it today, the implementation must
  add that return path rather than guess a boundary.

## Out of scope

- Changing Infinity Context behavior for non-Anthropic providers.
- Replacing the generic `compaction` capability or OpenAI compact endpoint.
- Making every runtime filter or deliberate prompt edit append-only.
- Supporting custom Anthropic-compatible endpoints without an explicit tested
  capability.
- Adding compaction controls to the public agent configuration.
- Custom compaction instructions or pausing after a compaction block.
- Rendering new UI. Existing generic compaction events are sufficient.
- Deleting, summarizing, or rewriting raw session events.
- Guaranteeing unlimited request or checkpoint byte size.

## Validation criteria

Implementation is complete only when all checks pass:

1. **Full-prefix regression:** an Anthropic wire test runs at least three turns
   with Infinity Context configured so the legacy hidden count would change.
   For each adjacent request pair, serialize `body["messages"]` and assert that
   the earlier complete array is byte-identical to the same-length prefix of
   the later array. The fixture includes a compaction block.
2. **No moving markers:** the same test asserts that native mode has top-level
   `cache_control`, has no message-level `cache_control`, and sends
   `compact-2026-01-12` plus `compact_20260112`.
3. **Streaming fidelity:** Anthropic SSE fixtures split readable compaction
   content across multiple `compaction_delta` events and include
   `encrypted_content`. The completed native block and next request replay are
   exact.
4. **Restart fidelity:** a reason-atom/storage test persists a compaction
   response, reconstructs the atom and store, and proves the next request uses
   the same prefix plus only events after the durable checkpoint boundary.
5. **Checkpoint failure:** injected write failure and a payload above 32 MiB
   both leave the raw completed response canonical. The next request rebuilds
   the exact prefix from raw events.
6. **Lossless history:** `query_history`, conversation retrieval, export, and a
   fork still return pre-compaction raw messages.
7. **Eligibility matrix:** profile tests cover every enabled family, dated
   aliases, and `[1m]` variants. They explicitly reject Opus 5.5, Haiku, unknown
   families, custom endpoints, and prompt-rewriting hooks. A case with both
   Infinity Context and generic compaction enabled proves no local reducer
   rewrites the native Anthropic prefix.
8. **Fallback:** an unsupported model sends no compaction beta or
   `context_management`, retains the counted legacy notice, and emits the
   fallback reason. A pre-stream beta rejection retries legacy once. A failure
   after native output does not retry with rewritten history.
9. **Cache telemetry:** a mocked two-turn response proves cache-read and
   cache-creation token buckets remain disjoint and compaction lifecycle
   metadata contains no prompt content.
10. **Commands:** run:
    - `cargo test -p everruns-anthropic --lib --all-features`
    - `cargo test -p everruns-builtins infinity_context`
    - `cargo test -p everruns-model-profiles`
    - `cargo test -p everruns-test-support --test reason_atom_test`
    - `just check-okf`
    - `just pre-push`

## References

- [Anthropic compaction](https://platform.claude.com/docs/en/build-with-claude/compaction)
- [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- [Anthropic cache diagnostics](https://platform.claude.com/docs/en/build-with-claude/cache-diagnostics)
- [Anthropic context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing)
- [Infinity Context](infinity-context.md)
- [Compaction](compaction.md)
- [LLM Drivers](../foundations/llm-drivers.md)
