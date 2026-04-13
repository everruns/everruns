# Compaction

## Abstract

Context compaction manages conversations that exceed LLM context windows. Users building agents and harnesses must be able to choose between **native provider compaction** (e.g., OpenAI `/responses/compact`) and **our own strategies** (observation masking, LLM summarization). This spec defines the compaction capability, its configuration surface, the strategy cascade, user-visible feedback, and implementation plan.

## Design Principles

1. **User chooses.** Agent/harness builders select their compaction strategy. We don't force one approach.
2. **Native and ours coexist.** Native provider compaction (opaque, high-fidelity) and our strategies (transparent, provider-agnostic) are both first-class options.
3. **Compaction ≠ summarization.** Compaction strips reproducible information (tool outputs, file contents). Summarization is lossy compression of irreducible information. Prefer compaction; use summarization as fallback.
4. **Proactive, not reactive.** Compact before hitting limits, not after `RequestTooLarge`.
5. **Users must know.** Every compaction emits events and renders in the UI.

## Current State

### What Exists

**OpenResponses Protocol Compaction** (`crates/core/src/atoms/reason.rs:839-953`):
- Reactive only — triggers on `RequestTooLarge` error
- Calls `LlmDriver::compact()` — only OpenAI Responses API supports it today
- Records `LlmCompactionInfo` as metadata on `llm.generation` event
- No SSE event, no UI indicator, no TS types

**Infinity Context** (`crates/core/src/capabilities/infinity_context.rs`):
- Separate, optional capability — not part of compaction. Trims messages + provides `query_history` tool.
- Complementary to compaction but independent. See `specs/infinity-context.md`.

### What's Missing

| Gap | Impact |
|-----|--------|
| No strategy selection | Users can't choose native vs our own |
| No proactive compaction | Waits for error, adds latency |
| No observation masking | Cheapest strategy not available |
| No LLM summarization | Non-OpenAI providers have no fallback |
| No SSE events | No real-time feedback |
| No UI indicator | Users don't know compaction happened |
| No TS types for `LlmCompactionInfo` | Frontend can't render compaction data |
| `supports_compact()` not exposed | Users can't see what their provider supports |

## Compaction Strategies

### Strategy Enum

```rust
pub enum CompactionStrategy {
    /// Use provider's native compact endpoint (e.g., OpenAI /responses/compact).
    /// Highest fidelity, opaque output. Only works if LlmDriver::supports_compact().
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

**No compaction capability configured → existing behavior (reactive native-only on error).**

### Default Behavior

When the `compaction` capability is present with no config (or `{}`):

| Setting | Default | Rationale |
|---------|---------|-----------|
| `strategy` | `auto` | Adapts to provider |
| `proactive` | `true` | Don't wait for errors |
| `budget_percent` | `0.85` | 15% headroom |
| `keep_recent_tool_outputs` | `2` | Recent context stays verbatim |
| `summarization.model` | `null` (same model) | Simplest default |
| `summarization.preserve` | `["decisions", "files_modified", "errors", "current_plan"]` | Key agent context |

## Events

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

Replaces old tool outputs with one-line summaries, keeping the N most recent verbatim. See `crates/core/src/capabilities/compaction.rs` for the masking algorithm.

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

See `crates/core/src/capabilities/compaction.rs` for implementation (`PROTECTED_TOOL_NAMES`, `is_protected_tool_result`).

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

## Implementation Plan

### Phase 1: Compaction Capability + Events

1. Create `CompactionConfig` types in `crates/core/src/capabilities/compaction.rs`
2. Register `compaction` capability in `CapabilityRegistry`
3. Add `context.compacting` / `context.compacted` event types to `crates/core/src/events.rs`
4. Refactor `ReasonAtom` compaction flow to:
   - Read compaction config from collected capabilities
   - Emit SSE events
   - Support strategy selection
5. Add `LlmCompactionInfo` + event types to UI TypeScript types
6. Render compaction divider in chat UI

### Phase 2: Observation Masking

1. Implement `ObservationMaskingFilter` in `crates/core/src/message_filter.rs`
2. Wire into compaction capability's `message_filter_provider()`
3. Add `observation_masking` as a strategy in `ReasonAtom`
4. Integrate into `auto` cascade (runs before native/summarization)

### Phase 3: Proactive Compaction

1. Add token estimation to message assembly in `ReasonAtom`
2. Check against `budget_percent` threshold before LLM call
3. Trigger cascade proactively instead of waiting for `RequestTooLarge`
4. Keep reactive path as fallback (config: `proactive: false`)

### Phase 4: Summarization

1. Implement `SummarizationCompactor` — takes messages + config, returns summary
2. Use agent's model by default, allow override via `summarization.model`
3. Integrate as final cascade step in `auto` strategy
4. Add as standalone strategy option

### Phase 5: Hierarchical Memory + Metrics

1. Implement hot/warm/cold tier management
2. Wire cold tier to Infinity Context's `query_history` when both capabilities enabled
3. Add `SessionCompactionMetrics` tracking
4. Display metrics in session UI

## References

- [Anthropic: Compaction API](https://platform.claude.com/docs/en/build-with-claude/compaction)
- [Anthropic: Effective Context Engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [JetBrains: The Complexity Trap (NeurIPS 2025)](https://arxiv.org/abs/2508.21433)
- [Microsoft: LLMLingua](https://llmlingua.com/llmlingua.html)
- [NAACL 2025: Prompt Compression Survey](https://aclanthology.org/2025.naacl-long.368.pdf)
- [OpenAI: GPT-5.2 Codex Context Compaction](https://openai.com/index/introducing-gpt-5-2/)
- [Google ADK: Context Compaction](https://google.github.io/adk-docs/context/compaction/)
- `specs/infinity-context.md` — complementary capability (optional, independent)
- `specs/events.md` — event schema
- `specs/capabilities.md` — capability system
- `crates/core/src/llm_driver_registry.rs` — `LlmDriver` trait with `supports_compact()` / `compact()`
- `crates/core/src/openresponses_protocol.rs` — `CompactRequest` / `CompactResponse` types
