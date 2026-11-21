---
type: Specification
title: "Compaction"
description: "Context compaction capability."
tags:
  - everruns
  - runtime-resources
---
# Compaction

## Managed continuity

`compaction` is the public long-session capability. Its compatibility dependency
supplies `query_history` and bootstrap loading; builders no longer need to enable
`infinity_context` separately. The legacy entry used alone remains a free window
policy. Runtime coordination, checkpoint durability and provider replay remain
core responsibilities rather than independent capability reducers.

The [context planner](../../crates/core/src/engine/execution/reason/context_plan.rs)
folds contiguous bounded journal pages into portable continuity. Its checkpoint
namespace and format are independent of provider/model-native records, sharing
the existing encrypted persistence and atomic installation path. Coverage uses
exact message-event cursors, never a masked or candidate-limited prompt. Successful
summary candidates persist separately before canonical installation. Their source
fingerprint permits reuse after installation failure; candidates do not advance
the assembly floor. Arrivals after the fixed snapshot remain in the suffix.

Generated continuity is reference data. Active skill source text is carried
verbatim outside generated prose, while current instructions, permissions and
runtime authority continue to come from current execution state. The summarizer
must finish without truncation or tool calls. Its paid usage is a separate traced
generation. Mechanical validity does not establish semantic fidelity; long-session
replay qualification is required.

Native epochs preserve canonical output order and their provider compatibility.
They bypass competing model-view masking. Portable summaries may bridge a model
change; opaque native content is never translated. Current working-context size
and raw evidence since the checkpoint trigger reduction; cumulative lifetime
usage alone cannot rearm a checkpoint. See the [public contract](../../docs/advanced/compaction.md).

The fallback reason is runtime telemetry. The model request records when
provider-managed reduction falls back to the portable path, and the outgoing
provider options contain only options the provider can use.

## Inspection and defaults

The shared Base preset supplies compaction to its children; Framework's
implicit harness supplies the same policy when builtins are compiled in.
Explicit custom harnesses retain control over their capability lists.
[Status projection](../../crates/core/src/compaction_status.rs) shares policy
resolution with execution and exposes metadata rather than saved content.
The [API context report](../../crates/server/src/domains/sessions/commands/context_report.rs)
measures the last main request, excluding separately traced summary and native work. It
reports missing measurements explicitly, and distinguishes unreported remote
store availability from an inspected empty store. Neither API nor Framework
inspection initiates a compaction. Backend availability is not crash durability.

## Terminal reduction failures

The Framework's former tiny-window trim-floor test exposed repeated actor
recovery of deterministic planner failures. Source-budget violations, rejected
summaries and non-reducing summaries are terminal configuration failures;
checkpoint-store failures keep their storage classification so candidate retry
remains available. A `RequestTooLarge` leaving the reason phase is terminal:
that phase already attempted permitted recovery. Framework tests bound execution
of an irreducible input and check a portable read floor across many turns.

## Cache and provenance regression analogies

[Eve #6498](https://github.com/vercel/eve/pull/6498) motivates testing persistent
announcements together with ephemeral context. The local continuation regression
in [context update tests](../../crates/core/src/engine/execution/reason/context_updates/tests.rs)
reproduced a catalog update following live-only facts. Recorded updates now
precede that volatile tail; their driver-request prefix survives a tool step and
the next turn, including when portable continuity is present. This checks request
shape, not provider cache-hit rates. A separate regression excludes runtime-marked
portable continuity from facts-bearing inputs: its restore-time timestamp must
not regenerate historical facts when the raw suffix starts with an answer.
An ordinary user quoting the same text remains a facts-bearing input.

[PydanticAI #13615](https://github.com/pydantic/pydantic-ai/pull/13615) motivates
preserving non-user provenance during reinjection. Everruns has no general
user/tool/model `pin()` API: its exact pins come from recorded `activate_skill`
results. [Continuity tests](../../crates/core/src/engine/execution/reason/context_plan_tests.rs)
verify that quoted pin markers from users, model text and file results do not
create pins through repeated summaries, and that restored continuity remains
framed reference data. No equivalent pin-selection bug was observed; these
structural checks do not prove that a model will resist instruction laundering.

## Request budget invariant

Proactive pressure includes model-visible tool declarations after discovery and
schema projection. Private deferred full schemas are excluded. Cache-stable
baseline/replay declarations are a separate compactable representation, not an
immutable prefix: fallback compaction restarts the epoch using the current
catalog. Native checkpoints and reactive history changes also invalidate the
old declaration baseline.

Trimming reserves the current catalog and keeps system instructions; fixed input
that reaches the estimated hard window fails explicitly rather than dropping
instructions or repeatedly compacting history. Estimates remain approximate and
can differ from provider tokenization and hosted-tool scaffolding. See the
[request budget implementation](../../crates/core/src/engine/execution/reason/request_budget.rs)
and [public behavior](../../docs/advanced/compaction.md).

## Native epochs and replay scope

Native output remains an ordered provider-owned replacement. A subsequent native
request composes that replacement with the complete raw suffix and clears the
old response handle when switching to a standalone checkpoint. Current system
instructions are assembled separately. Provider/model/format compatibility is
checked before replay; portable continuity is reconstructed from journal data
when a compatible native checkpoint is unavailable.

Supported reasoning-update models also retain their effort epoch across
compaction and reassert the effective effort after the replacement boundary.
Configuration updates never move across semantic items. Local strategies retain
request-level effort controls without translating opaque native state. The
contracts live in [reasoning replay](../../crates/core/src/engine/execution/reason/reasoning_updates.rs),
[native reduction](../../crates/core/src/engine/execution/reason/compaction.rs), and
[wire regressions](../../crates/contracts/tests/openresponses_protocol_wire.rs).

A reasoning artifact's upstream format tag does not identify the configured
route that can replay it. The [transcript projection](../../crates/core/src/engine/execution/reason/transcript.rs)
checks recorded driver, model and configured provider identity. Opaque artifacts
with missing or incompatible provenance are omitted from requests, while source
events stay intact. This does not make a native checkpoint portable or establish
compatibility after credentials change beneath the same configured identity.

## Current input and exchange boundaries

The current triggering input remains verbatim on local portable paths, even when
a long tool loop moves its journal record inside the summarized prefix. Identity
comes from the execution's recorded input ID, not a generated paraphrase or the
last user-channel message: synthetic facts can also use that channel. The runtime
restores the original input and preserves its assembled actor label and media
through request-local trimming.

Recent-history retention is a preference rather than a reason to split a completed
parallel exchange. Prefix selection accounts for the existing continuity record
and may cross the preferred recent-message floor to obtain a useful reduction.
An incomplete exchange or a complete batch that cannot fit the summary source
window still fails visibly. A request-limit stop can leave an unfinished raw
exchange; this remains a recovery limitation rather than permission to fabricate
successful results. See the [planner](../../crates/core/src/engine/execution/reason/context_plan.rs)
and [public Framework regressions](../../crates/everruns/tests/host/integration/compaction_test.rs).

## Request-local masking and cache behavior

Deterministic masking and hierarchical transformations are prompt projections;
they do not establish durable coverage of omitted journal records. Protected
skill sources come from actual recorded activation results. Quoted markers do
not create provenance or durable authority. Last-resort trimming can still fail
to fit irreducible instructions or exact skill sources.

Masking boundaries advance in batches to reduce prefix churn. An unchanged
boundary can reuse a provider cache; a masking event does not establish a cache
miss or a separate paid call. Likewise, a structural prefix regression test is
not a measured provider cache-hit improvement. Exact policy and transformations
belong in the [capability](../../crates/core/src/builtins/compaction.rs) and
[cost-control projection](../../crates/core/src/builtins/compaction/cost_control.rs).

## Measurement and cost

Distinguish retained-window measurements, request estimates, provider-reported
generation usage, and monetary attribution. Explicit native compaction's generated
tokens are billed work rather than the size of its resulting window. The driver
preserves those counters; reduction validation uses serialized bytes when window
token counts are unavailable. Byte reduction of opaque state is a heuristic, not
proof of semantic fidelity or the next request's token count.

Portable summaries and native calls have separate generation events, including
completed responses rejected before checkpoint installation. Main-generation usage
never includes that spend again. Both paid paths retain configured provider
identity independently of the driver name for hosted account attribution.
Native provider-reported cost and price-table
estimates also remain attributable through compaction metadata. Effective spend
prefers reported cost and otherwise uses an available estimate without marking it
actual. See [native accounting](../../crates/core/src/engine/execution/reason/native_compaction.rs). Missing counters or model pricing remain unknown; neither estimates nor
preserved usage establish invoice reconciliation. Inspection must not manufacture missing
measurements or present a message-count change as a token-saving measurement.

Initial backfill may need multiple calls. Subsequent portable work is incremental
in new source history. Summary requests share the actual summary model's context
window with their output allowance; bounded source selection reserves that space
before sending a request. Configuration, event shapes and defaults are owned by
[source](../../crates/core/src/builtins/compaction.rs) and the
[public guide](../../docs/advanced/compaction.md), not duplicated here.

## Qualification boundaries

Synthetic long-session tests establish cursor, restart, retrieval and resource
behavior. Live snapshot replays additionally test selected task-state, correction
and approval facts across provider changes. Neither is a universal fidelity,
multimodal, cache, latency or cost guarantee. Missing attachment bytes and
unverified live infrastructure state must remain explicit unknowns.

The [semantic probe](../../crates/core/benches/compaction_semantics.rs) and
[continuity lifecycle regressions](../../crates/core/src/engine/execution/reason/context_plan_tests.rs)
provide repeatable evidence. Model judgments supplement source review rather than
replacing it. Public APIs expose safe policy/checkpoint metadata, never checkpoint
payloads or provider-encrypted reasoning material.
