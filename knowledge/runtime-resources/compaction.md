---
type: Specification
title: "Compaction"
description: "Context compaction capability."
tags:
  - everruns
  - runtime-resources
---
# Compaction

## Abstract

Context compaction manages conversations that exceed LLM context windows. Users building agents and harnesses must be able to choose between **native provider compaction** (e.g., OpenAI `/responses/compact`) and **our own strategies** (observation masking, LLM summarization). This spec defines the compaction capability, its configuration surface, the strategy cascade, user-visible feedback, and implementation plan.

## Design Principles

1. **User chooses.** Agent/harness builders select their compaction strategy. We don't force one approach.
2. **Native and ours coexist.** Native provider compaction (opaque, high-fidelity) and our strategies (transparent, provider-agnostic) are both first-class options.
3. **Compaction ≠ summarization.** Compaction strips reproducible information (tool outputs, file contents). Summarization is lossy compression of irreducible information. Prefer compaction; use summarization as fallback.
4. **Proactive, not reactive.** Compact before hitting limits, not after `RequestTooLarge`.
5. **Users must know.** Every compaction emits events and renders in the UI.

## Model View Versus Storage

Session storage remains lossless: tool results, file reads, and exec outputs are stored as events/messages exactly as produced. When the `compaction` capability is configured, it contributes a model-view provider that builds a separate **model view** from those stored messages before provider serialization.

The model view applies generic cost-control masking according to the configured `compaction` capability. This masking replaces stale bulky tool results with compact summaries while preserving message/tool-call structure and the most recent tool results verbatim. Builders can tune or disable this via `cost_control`.

Provider cache telemetry feeds this same model-view step. If the previous call reports high uncached input or a low cache-read ratio, the model view may mask older tool results earlier so repeated full-history prompts do not keep paying for cache misses.

For native-capable drivers, proactive policy also treats cumulative uncached
input and the raw bytes of tool results accumulated since the latest durable
checkpoint as cost pressure. Either signal can request a checkpoint before the
model view reaches the context-window threshold, but only when the current
prompt is itself non-trivial. This marginal floor avoids spending a compact
call on short follow-ups after a long session. The same checkpoint re-arming,
lineage-aware retry watermark, and failure fallback used by window pressure
apply; cost pressure does not create a second compaction lifecycle.

### Durable replacement checkpoints

Compaction that replaces a history prefix MUST survive later turns and durable
worker restart. The replacement is stored as a session checkpoint, separate
from the immutable event log. Raw session events remain the lossless source for
conversation APIs, export, audit, and `query_history`.

A checkpoint records an opaque replacement context, a compatibility identity
(provider, model, and format version), and the event-sequence boundary of the
raw history used to create it. Model input is assembled as the latest
compatible checkpoint followed by message events strictly after that boundary.
An incompatible checkpoint is ignored and the model view is rebuilt from raw
history; provider-native content is never translated through a magic text
message or sent to a different provider/model.

Checkpoint installation is atomic and monotonic. A completed compaction may
replace the current checkpoint only when its source boundary is not older than
the installed boundary. Messages committed while compaction is in flight have
higher event sequences and therefore remain in the raw suffix. A failed compact
call or failed checkpoint write leaves the previous checkpoint canonical and
does not emit a successful compaction event.

Proactive `auto` and `native` policy uses the driver's native compact operation
when it is supported and installs the result through this same checkpoint path;
reactive and proactive compaction do not have separate durability semantics. A
fresh checkpoint is disarmed for proactive replacement until a meaningful raw
suffix has accumulated. Native output is accepted only when provider token
usage, or serialized byte size when token usage is unavailable, proves a
material reduction of at least 5%, with a 32-unit absolute floor for small
measurements. A smaller effect does not replace the checkpoint or emit
`context.compacted`. Every proactive native attempt also records its session,
provider/model, and raw source boundary in a process-lifetime retry watermark.
An ineffective or failed attempt is not repeated for an unchanged or tiny
suffix; estimated input growth of at least 4,096 tokens and 5% of the attempted
input re-arms it. Source sequence supplies monotonic identity while a transcript
prefix fingerprint prevents an abandoned branch's watermark from suppressing
the selected branch. The process-local map is capped at 4,096
session/provider/model entries with oldest-entry eviction. This retry-control
marker is not a semantic checkpoint and is never exposed as one.
Watermark lookup and recording fail open with a warning: a transient retry-map
or adapter failure must never block the normal model turn.

When a checkpoint is compacted again, its ordered opaque output is converted to
compact input without interpretation and prepended to the raw suffix. This
composition applies to proactive re-arming and reactive `RequestTooLarge`
recovery; checkpoint N+1 must semantically contain checkpoint N plus every
suffix item through its new source boundary.

Provider-native checkpoint payloads are sensitive opaque data and MUST be
encrypted at rest. They are available only to the internal runtime storage
boundary. Public events and APIs expose, at most, the checkpoint identifier,
strategy, timing, and size/token metrics; they never expose encrypted provider
content or its plaintext.

Provider-neutral summarization can use the same replacement-checkpoint
contract with a distinct format and compatibility policy. It does not require
changing raw event storage or provider-native serialization.

### Tool-call structural integrity

Prompt-facing reduction preserves tool calls and results as atomic protocol
structure. If a reducer removes a result from a parallel batch, it also removes
that call from the visible assistant call set while leaving complete protected
pairs intact. A stateless model view likewise removes results whose calls are no
longer visible. Stateful Responses continuations are the sole exception: a
result-only delta may refer to a call held by `previous_response_id`.

This invariant applies to proactive and reactive trimming, summary boundaries,
hierarchical tiers, provider-compacted output, and infinity-context composition.
Reducers only change prompt-facing copies: stored session history remains
lossless, and reducers never invent successful tool results.

### Native compact context handoff

Native compact output is a standalone, provider-owned context checkpoint. The
runtime preserves the returned item array as an ordered typed value and does
not translate encrypted items into messages, reorder them, or run generic
message/tool pruning over them. The next matching provider request uses that
array directly as `input`; `previous_response_id` is cleared because the
standalone checkpoint replaces the earlier server-side continuation chain.

The compact request itself uses reconstructed standalone `input` without
`previous_response_id`, including for a stateful Responses continuation. This
preserves the fresh request delta when the retry replaces the server-side
continuation chain with the compact output. System instructions remain a
separate request field.

Opaque compact content is provider transport state, not a public event payload.
Compaction events expose counts, strategy, duration, and usage metadata only.

Stateful `previous_response_id` requests are deltas over provider-held context.
The reconstructed lossless transcript is therefore not a valid local pressure
measurement for proactive policy. In the absence of authoritative provider
usage for that server-side context, the runtime skips local proactive
compaction and retains reactive `RequestTooLarge` recovery using reconstructed
standalone input.

Observation masking remains a model-view cost optimization. It may reduce an
outbound request without replacing semantic history, and by itself does not
emit a successful durable `context.compacted` event.

## Current State

### What Exists

**Compaction capability** (`crates/builtins/src/compaction.rs`):
- Configured explicitly through the `compaction` capability.
- Contributes the prompt-facing model-view provider that masks stale bulky tool results before provider serialization.
- Supports proactive budget checks, reactive `RequestTooLarge` recovery, observation masking, native provider compaction when available, summarization, and last-resort trimming.
- Emits `context.compacting` / `context.compacted` events and records `LlmCompactionInfo` on `llm.generation` when native provider compaction runs.

**Infinity Context** (`crates/builtins/src/infinity_context.rs`):
- Separate, optional capability — not part of compaction. Keeps a recent window + provides the `query_history` tool.
- **Not freely composable with compaction.** Infinity context evicts during message loading, before compaction runs, so enabling both naively means compaction only sees the recent window. Compaction is the stronger primary strategy (always-present summary); infinity context is a pull-based backstop. When both are enabled, infinity context defers token-budget eviction to compaction. See `knowledge/runtime-resources/infinity-context.md`.

### Current Caveats

| Gap | Impact |
|-----|--------|
| Provider capability visibility | Users cannot yet see every provider's native compaction support in all product surfaces. |
| UI polish | The backend emits compaction events, but timeline rendering may still need product-specific display work. |
| Token telemetry precision | Some compaction events report message counts rather than exact token deltas when the strategy does not make a provider call. |

## Compaction Strategies

### Strategy Enum

```rust
pub enum CompactionStrategy {
    /// Use provider's native compact endpoint (e.g., OpenAI /responses/compact).
    /// Highest fidelity, opaque output. Only works if ChatDriver::supports_compact().
    Native,

    /// Strip tool outputs from older turns, replace with one-line summaries.
    /// Cheapest. Preserves reasoning chain. Provider-agnostic.
    ObservationMasking,

    /// Use LLM to summarize older turns into structured summary.
    /// Provider-agnostic. Lossy but works everywhere.
    Summarization,

    /// Observation masking first, then native compact if still over budget.
    /// Falls back to summarization if native not available.
    Auto,
}
```

### Strategy Comparison

| Strategy | Cost | Fidelity | Provider Requirement | Best For |
|----------|------|----------|---------------------|----------|
| `native` | Low (API call) | Highest (encrypted blobs) | `supports_compact()` = true | OpenAI Responses API users |
| `observation_masking` | Zero (no LLM call) | High (reasoning preserved) | None | All providers, cost-sensitive |
| `summarization` | Medium (extra LLM call) | Medium (lossy) | None | Long conversations, any provider |
| `auto` | Varies | Best available | None | Default — adapts to provider |

### `auto` Cascade

```
1. Observation masking (always, free)
   ↓ still over budget?
2. Native compact (if driver supports it)
   ↓ not supported or still over budget?
3. Summarization (LLM call, lossy)
   ↓ still over budget?
4. Aggressive trim (drop oldest, last resort)
```

## Configuration

### Capability Config

Compaction is configured as a capability on agents and harnesses, following the existing `AgentCapabilityConfig` pattern:

```json
{
  "capabilities": [
    {
      "ref": "compaction",
      "config": {
        "strategy": "auto",
        "proactive": true,
        "budget_percent": 0.85,
        "observation_masking": {
          "keep_recent_tool_outputs": 2,
          "summary_format": "one_line"
        },
        "summarization": {
          "model": null,
          "preserve": ["decisions", "files_modified", "errors", "current_plan"],
          "instructions": null
        }
      }
    }
  ]
}
```

### CompactionConfig

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Which strategy to use. Default: Auto.
    #[serde(default = "default_strategy")]
    pub strategy: CompactionStrategy,

    /// Compact proactively at budget_percent, not just on RequestTooLarge.
    #[serde(default = "default_proactive")]
    pub proactive: bool,  // default: true

    /// Trigger proactive compaction at this fraction of context budget.
    /// Only used when proactive = true.
    #[serde(default = "default_budget_percent")]
    pub budget_percent: f32,  // default: 0.85

    /// Observation masking settings.
    #[serde(default)]
    pub observation_masking: ObservationMaskingConfig,

    /// Summarization settings. Only used when strategy includes summarization.
    #[serde(default)]
    pub summarization: SummarizationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationMaskingConfig {
    /// Number of recent tool outputs to keep verbatim.
    #[serde(default = "default_keep_recent_tool_outputs")]
    pub keep_recent_tool_outputs: usize,  // default: 2

    /// Format for masked tool output summaries.
    #[serde(default)]
    pub summary_format: MaskingSummaryFormat,  // default: OneLine
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MaskingSummaryFormat {
    /// `[tool: read_file("src/main.rs") → 245 lines, OK]`
    OneLine,
    /// Keep first and last 3 lines of output
    HeadTail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationConfig {
    /// Model to use for summarization. None = same model as agent.
    #[serde(default)]
    pub model: Option<String>,

    /// What to preserve in summaries.
    #[serde(default = "default_preserve")]
    pub preserve: Vec<String>,

    /// Custom instructions appended to summarization prompt.
    /// Similar to Claude Code's `/compact <instructions>`.
    #[serde(default)]
    pub instructions: Option<String>,
}
```

Cost-control masking is also part of compaction. It runs before provider
serialization and replaces older bulky tool results with small structured
summaries when repeated full-history prompts would otherwise keep paying for
stale `read_file`, exec, listing, or search output. It is enabled by default
when compaction is enabled, keeps the most recent tool results verbatim, and can
also trigger from prior usage signals when cache reuse is poor. See
[`crates/builtins/src/compaction.rs`](../../crates/builtins/src/compaction.rs)
for the exact configuration fields and defaults.

### Config Examples

**"I want native OpenAI compaction only":**
```json
{ "ref": "compaction", "config": { "strategy": "native" } }
```

**"I want our own strategies, no native":**
```json
{ "ref": "compaction", "config": { "strategy": "observation_masking" } }
```

**"Full auto, be proactive":**
```json
{ "ref": "compaction", "config": { "strategy": "auto", "proactive": true } }
```

**"Summarize with specific instructions, use a fast model":**
```json
{
  "ref": "compaction",
  "config": {
    "strategy": "summarization",
    "summarization": {
      "model": "claude-haiku-4-5-20251001",
      "instructions": "Preserve all API endpoint decisions and error patterns"
    }
  }
}
```

**No compaction capability configured → no Everruns compaction or masking runs.**
Provider `RequestTooLarge` errors propagate unless the agent/harness enables the
`compaction` capability.

### Default Behavior

When the `compaction` capability is present with no config (or `{}`):

| Setting | Default | Rationale |
|---------|---------|-----------|
| `strategy` | `auto` | Adapts to provider |
| `proactive` | `true` | Don't wait for errors |
| `budget_percent` | `0.85` | 15% headroom |
| `keep_recent_tool_outputs` | `2` | Recent context stays verbatim |
| Cost-control masking | Enabled | Prevent stale bulky tool results from being resent verbatim in every request |
| Cost-control durable checkpoint | 100,000 cumulative uncached input tokens or 256 KiB raw tool results, with an 8,192-token current-prompt floor | Bound repeated prompt cost before 85% occupancy without compacting trivial turns |
| `summarization.model` | `null` (same model) | Simplest default |
| `summarization.preserve` | `["decisions", "files_modified", "errors", "current_plan"]` | Key agent context |

## Events

### Cost

Native compaction is a real billable model call. Providers that report per-call
cost inline (OpenAI-compatible gateways returning `usage.cost`) have it carried
through the compaction response onto the generation event two ways:

- `metadata.compaction.cost_usd` keeps it separately attributable, so an
  operator can see how much of a turn's spend was compaction rather than
  generation.
- It is also folded into `metadata.usage.actual_cost_usd`, because the usage
  listener reads only `metadata.usage` — that is the sole path to budget debits
  and `llm_generations`.

When the generation itself reports no usage, the compaction cost creates a usage
record rather than being dropped. Providers that do not report a cost leave both
fields absent, matching the ordinary generation paths.

### SSE Event Types

Two new event types for real-time compaction feedback:

```
event: context.compacting
data: {
  "id": "evt_...",
  "type": "context.compacting",
  "ts": "2026-03-15T...",
  "session_id": "session_...",
  "data": {
    "reason": "proactive_budget",
    "strategy": "auto",
    "tokens_before": 180000
  }
}

event: context.compacted
data: {
  "id": "evt_...",
  "type": "context.compacted",
  "ts": "2026-03-15T...",
  "session_id": "session_...",
  "data": {
    "checkpoint_id": "cmp_...",
    "strategy_used": "observation_masking+native",
    "tokens_before": 180000,
    "tokens_after": 95000,
    "duration_ms": 2300,
    "steps": [
      { "strategy": "observation_masking", "tokens_after": 140000, "duration_ms": 12 },
      { "strategy": "native", "tokens_after": 95000, "duration_ms": 2288 }
    ]
  }
}
```

`checkpoint_id` is present when a durable replacement checkpoint was
installed. Event data MUST NOT contain the checkpoint payload, provider-native
encrypted content, or the at-rest ciphertext.

**Reason enum:** `proactive_budget` | `request_too_large` | `manual`

The `steps` array shows the cascade — which strategies ran and their individual contribution. This is critical for debugging and tuning.

### Existing Event Enhancement

`LlmCompactionInfo` on `llm.generation` events remains for backward compatibility but becomes redundant once `context.compacted` events are adopted.

## UI

### Compaction Divider

When `context.compacted` fires, render in the chat timeline:

```
─────── Context compacted · 180K → 95K tokens ───────
```

Expandable on click to show:
- Strategy used (with cascade steps if `auto`)
- Duration
- Tokens saved
- Reason (proactive vs error recovery)

### TypeScript Types

```typescript
export interface LlmCompactionInfo {
  compacted: boolean;
  input_tokens_before?: number;
  input_tokens_after?: number;
  duration_ms?: number;
}

export interface ContextCompactingEvent {
  reason: "proactive_budget" | "request_too_large" | "manual";
  strategy: "native" | "observation_masking" | "summarization" | "auto";
  tokens_before: number;
}

export interface ContextCompactedEvent {
  strategy_used: string;
  tokens_before: number;
  tokens_after: number;
  duration_ms: number;
  steps: CompactionStep[];
}

export interface CompactionStep {
  strategy: string;
  tokens_after: number;
  duration_ms: number;
}
```

### LLM History Viewer

The existing `llm-history-viewer.tsx` component should display compaction info when present in `LlmGenerationMetadata.compaction`.

## Observation Masking

Replaces old tool outputs with one-line summaries, keeping the N most recent verbatim. See `crates/builtins/src/compaction.rs` for the masking algorithm.

### Tool-Aware Masking (Tier 3)

When tool type is known, apply type-specific compression:

| Tool | Mask Strategy |
|------|--------------|
| `activate_skill` | **Never mask** — protected, always kept verbatim |
| `read_file` | Keep path + line count: `[read_file("src/main.rs") → 245 lines]` |
| `bash` | Keep exit code + last 20 lines |
| `search` / `grep` | Keep matched file paths only |
| `write_file` | Keep path + operation: `[write_file("src/main.rs") → 245 lines written]` |
| `web_fetch` | Keep URL + status: `[web_fetch("https://...") → 200 OK, 12KB]` |

### Protected Tool Results

Tool results from `PROTECTED_TOOL_NAMES` (currently `activate_skill`) are exempt from all compaction strategies:

1. **Observation masking**: excluded from the maskable tool index — never replaced with summaries
2. **Aggressive trim**: budget reserved first; always kept even when other messages are dropped
3. **Hierarchical memory**: rescued from cold tier into output verbatim
4. **Summarization**: `skill_instructions` in default preserve list; prompt instructs LLM to include skill content verbatim

### Anchored Task Message

Aggressive trim also anchors the **first conversation message** (the original
task/goal), reserving its budget alongside protected tool results so it is never
dropped. Like infinity context's head anchor, losing the opening task leaves the
model unable to tell what it is doing once the window slides; the system prompt
is assembled separately and is already exempt.

See `crates/builtins/src/compaction.rs` for implementation (`PROTECTED_TOOL_NAMES`, `is_protected_tool_result`).

## Summarization

### Summarization Prompt

```xml
<task>
Summarize the following conversation history. The summary replaces these
messages in the agent's context window — it must contain everything the
agent needs to continue working.
</task>

<preserve>
- Decisions made and their rationale
- Files created, modified, or deleted (with paths)
- Errors encountered and how they were resolved
- Current plan and next steps
- Key user requirements not yet addressed
{{#if config.instructions}}
- {{config.instructions}}
{{/if}}
</preserve>

<format>
Produce a structured summary. Use sections. Be concise but complete.
Do not include tool output verbatim — reference files by path.
</format>

<messages>
{{messages}}
</messages>
```

### Summary Message

The summary replaces compacted messages as a single system message:

```rust
LlmMessage {
    role: LlmMessageRole::System,
    content: LlmMessageContent::Text(format!(
        "[CONVERSATION_SUMMARY]\n{}\n[/CONVERSATION_SUMMARY]",
        summary_text
    )),
    // ...
}
```

Marked with `[CONVERSATION_SUMMARY]` tags so future compaction rounds can identify and re-summarize if needed, but never recursively — a summary is only re-summarized if new context has accumulated around it.

## Hierarchical Memory (Tier 3)

Three tiers of conversation context:

| Tier | Messages | Content | Retention |
|------|----------|---------|-----------|
| **Hot** | Last ~20 | Full verbatim text | Always in context |
| **Warm** | Next ~100 | Observation-masked (tool outputs replaced) | In context if budget allows |
| **Cold** | All older | Summarized to key facts | Queryable via `query_history` if Infinity Context enabled |

Tier boundaries are configurable per agent. The `auto` strategy manages tier transitions automatically based on token budget.

## Session-Level Metrics

Track per session (stored as session metadata, queryable via API):

```rust
pub struct SessionCompactionMetrics {
    /// Total number of compaction events in this session
    pub compaction_count: u32,
    /// Total input tokens saved across all compactions
    pub total_tokens_saved: u64,
    /// Breakdown by strategy
    pub strategy_counts: HashMap<String, u32>,
    /// Total time spent compacting (ms)
    pub total_duration_ms: u64,
}
```

Displayed in session detail view and session list (as a subtle indicator when compaction has occurred).

## Implementation Status

Implemented pieces live in the capability and runtime assembly paths:

- `crates/builtins/src/compaction.rs` owns config parsing,
  cost-control model-view masking, observation masking, summarization helpers,
  and compaction metrics types.
- `crates/core/src/capabilities/mod.rs` exposes the generic
  `ModelViewProvider` hook so compaction remains capability-owned.
- `crates/engine/src/execution/reason.rs` invokes compaction only when the resolved
  capability config includes `compaction`; without it, context-limit errors are
  returned to the caller.
- `crates/core/src/events.rs` defines compaction events and generation metadata.

Remaining product work should focus on UI presentation and provider capability
visibility rather than changing the capability ownership model.

### Future Work

- Product UI for compaction timeline events and session-level metrics.
- Provider capability visibility for native compaction support.
- Deeper integration between hierarchical memory tiers and Infinity Context.

## References

- [Anthropic: Compaction API](https://platform.claude.com/docs/en/build-with-claude/compaction)
- [Anthropic: Effective Context Engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [JetBrains: The Complexity Trap (NeurIPS 2025)](https://arxiv.org/abs/2508.21433)
- [Microsoft: LLMLingua](https://llmlingua.com/llmlingua.html)
- [NAACL 2025: Prompt Compression Survey](https://aclanthology.org/2025.naacl-long.368.pdf)
- [OpenAI: GPT-5.2 Codex Context Compaction](https://openai.com/index/introducing-gpt-5-2/)
- [Google ADK: Context Compaction](https://google.github.io/adk-docs/context/compaction/)
- `knowledge/runtime-resources/infinity-context.md` — pull-based backstop capability; defers to compaction when both are enabled
- `knowledge/execution/events.md` — event schema
- `knowledge/execution/capabilities.md` — capability system
- `crates/provider/src/driver_registry.rs` — `ChatDriver` trait with `supports_compact()` / `compact()`
- `crates/provider/src/openresponses_protocol.rs` — `CompactRequest` / `CompactResponse` types
