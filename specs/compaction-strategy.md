# Compaction Strategy

## Abstract

Context compaction is how we handle conversations that exceed LLM context windows. Today we have two backend mechanisms (Infinity Context trimming and OpenResponses protocol compaction) but zero user-visible feedback. This spec defines our compaction strategy: what we do, what users see, and where we go next.

## Current State

### Two Backend Layers

#### Layer 1: Infinity Context (soft trim + query tool)

Opt-in capability per agent. See `crates/core/src/capabilities/infinity_context.rs`.

- Trims older messages when token count exceeds budget (default 100K tokens, ~70% of context)
- Always keeps min 10 recent messages
- Injects system notice only the LLM sees: *"Note: {N} earlier messages not shown"*
- Provides `query_history` tool so the LLM can pull historical context on-demand
- User sees **nothing** — trimming is invisible to humans

#### Layer 2: OpenResponses Compaction (emergency compression)

Automatic, transparent. See `crates/core/src/atoms/reason.rs`.

- Triggers **only on `RequestTooLarge` error** — reactive, not proactive
- Calls `/v1/responses/compact` on the LLM driver
- Replaces assistant messages + tool calls with encrypted opaque blobs
- Retries the LLM call with compacted context
- Records `LlmCompactionInfo` (tokens_before, tokens_after, duration_ms) as metadata on the `llm.generation` event

### What We Show: Nothing

| Aspect | Status |
|--------|--------|
| SSE event for compaction | None — only metadata on `llm.generation` |
| UI indicator during compaction | None |
| UI indicator after compaction | None — `LlmCompactionInfo` not in TS types |
| Progress bar/spinner | None |
| History trimming notice to user | None — only LLM sees it |
| Compaction metrics in session view | None |

### Gaps

1. **No real-time feedback** — Compaction can take seconds. User sees "thinking" with no explanation.
2. **No dedicated SSE event** — Compaction buried as optional metadata on `llm.generation`.
3. **No UI types** — `LlmGenerationMetadata` in `apps/ui/src/lib/api/types.ts` omits `compaction`.
4. **No visual indicator** — Claude Code shows "Context automatically compacted" divider. We show nothing.
5. **Infinity Context trimming invisible to humans** — LLM knows messages were trimmed, user doesn't.
6. **No compaction history** — Can't see how many times compaction occurred or total tokens saved.
7. **Reactive only** — Wait for `RequestTooLarge` error before compacting, adding latency.

## State of the Art (March 2026)

### Industry Approaches

| Product | Strategy | User Feedback |
|---------|----------|---------------|
| **Claude Code** | LLM summarization + `/compact` command + PreCompact hooks. Keeps 5 most recent files alongside summary. `clear_tool_uses` strips old tool outputs chronologically. | "Context automatically compacted" divider |
| **Cursor** | Auto-summarization at 100% context using flash model. `/compress` command. Saves chat history as files for later reference. | Minimal — auto-compresses silently |
| **Windsurf** | Context assembly pipeline with M-Query retrieval. Persistent Memories across sessions. No explicit compaction. | None visible |
| **OpenAI Codex** | `/responses/compact` endpoint producing opaque compressed representations. Up to 99.3% compression. | None documented |
| **Google ADK** | Built-in sliding-window summarization of agent workflow events. | Framework-level, not user-visible |

### Academic Insights

| Technique | Key Finding |
|-----------|-------------|
| **Observation masking** (JetBrains, NeurIPS 2025) | Stripping tool outputs matches LLM summarization quality at half the cost. LLM summarization can cause "trajectory elongation" — smoothing over failures makes agents not realize how stuck they are. |
| **LLMLingua family** (Microsoft, ACL 2024) | Token-level compression via perplexity scoring. 2-20x compression, minimal quality loss. LLMLingua-2 reframes as token classification, 3-6x faster. |
| **Hybrid masking + summarization** | Masking first, summarize only when necessary. 7-11% additional cost reduction over masking alone. |

### Emerging Consensus

1. **Compaction > summarization** — Strip information that exists in the environment and can be re-fetched (tool outputs, file reads) before resorting to lossy summarization.
2. **Observation masking is underrated** — Simple, cheap, preserves reasoning chains.
3. **Proactive > reactive** — Don't wait for overflow; compact at a budget threshold.
4. **Hybrid wins** — Multiple strategies at different context levels.

## Strategy

### Design Principles

1. **Compaction is not summarization.** Compaction strips reproducible information (tool outputs, file contents). Summarization is lossy compression of irreducible information (decisions, reasoning). We should prefer compaction and use summarization as a last resort.
2. **Users must know.** When context is modified, users should see it. Silent compaction erodes trust.
3. **Proactive, not reactive.** Compact before hitting the wall, not after crashing into it.
4. **Provider-agnostic.** Our compaction strategy should work regardless of whether the LLM driver supports `/responses/compact`.

### Tier 1: Visibility (low effort, high value)

#### 1a. SSE Events for Compaction

Add dedicated event types:

```
event: context.compacting
data: {"session_id":"...","reason":"token_budget_exceeded","tokens_before":180000}

event: context.compacted
data: {"session_id":"...","tokens_before":180000,"tokens_after":95000,"duration_ms":2300,"strategy":"openresponses_compact"}
```

Reason enum: `token_budget_exceeded` | `manual` | `proactive_budget`

Strategy enum: `openresponses_compact` | `observation_masking` | `summarization` | `infinity_context_trim`

These are **new event types**, not metadata on existing events. They fire as the compaction happens, enabling real-time UI updates.

#### 1b. UI Compaction Indicator

When `context.compacted` fires, render a visual separator in the chat timeline:

```
─────── Context compacted · 180K → 95K tokens ───────
```

Subtle, non-intrusive, horizontally centered. Similar to Claude Code's approach. Clicking expands to show strategy, duration, and tokens saved.

#### 1c. Infinity Context Trim Notice

When Infinity Context excludes messages, show a collapsible indicator at the top of the chat:

```
┌─────────────────────────────────────────────┐
│ 198 earlier messages not shown              │
│ The agent can search them with query_history│
└─────────────────────────────────────────────┘
```

This is the *user-facing* equivalent of the system notice we already inject for the LLM.

#### 1d. TypeScript Types

Add `compaction` field to `LlmGenerationMetadata` in `apps/ui/src/lib/api/types.ts`:

```typescript
export interface LlmCompactionInfo {
  compacted: boolean;
  input_tokens_before?: number;
  input_tokens_after?: number;
  duration_ms?: number;
}

export interface LlmGenerationMetadata {
  // ... existing fields ...
  compaction?: LlmCompactionInfo;
}
```

### Tier 2: Intelligence (medium effort)

#### 2a. Observation Masking (before compaction)

Based on JetBrains research: strip tool outputs from older turns before attempting full compaction. Tool outputs (file reads, search results, command outputs) are the largest context consumers and can be re-fetched.

Algorithm:
1. Keep last N tool outputs verbatim (N = 5)
2. Replace older tool outputs with one-line summaries: `[tool: read_file("src/main.rs") → 245 lines, OK]`
3. Preserve all user messages, assistant reasoning, and tool *calls* (just not results)

This is cheaper than LLM summarization and preserves the reasoning chain.

#### 2b. Proactive Compaction

Don't wait for `RequestTooLarge`. Monitor token count and compact at 85% of context budget:

```
if estimated_tokens > context_budget * 0.85 {
    // Apply observation masking first
    // If still over budget, apply compaction
    // Emit context.compacting / context.compacted events
}
```

This eliminates the error/retry cycle and makes compaction predictable.

#### 2c. Conversation Summarization (provider-agnostic fallback)

When the LLM driver does NOT support `/responses/compact`, use the LLM itself to summarize older turns:

- Generate a structured summary preserving: decisions made, files modified, errors encountered, current plan
- Replace summarized messages with a single system message containing the summary
- Mark the summary as non-compactable to prevent recursive summarization

### Tier 3: Advanced (higher effort, future)

#### 3a. Hierarchical Memory

Three tiers of conversation context:

| Tier | Content | Retention |
|------|---------|-----------|
| **Hot** (last ~20 messages) | Full verbatim text | Always in context |
| **Warm** (last ~100 messages) | Paragraph-level summaries | In context if budget allows |
| **Cold** (all older messages) | Key facts + decisions only | Queryable via `query_history` |

#### 3b. Smart Tool Output Compression

Tool-type-aware compression:
- `read_file` → Keep only the lines the LLM actually referenced in its response
- `bash` → Keep exit code + last 20 lines of output
- `search` → Keep only the matched file paths, not full content
- `write_file` → Keep the file path + line count, drop the full content

#### 3c. Session-Level Compaction Metrics

Track and display per-session:
- Total compaction events
- Total tokens saved
- Compaction strategy distribution
- Time spent in compaction

## Implementation Plan

### Phase 1: Visibility
1. Add `context.compacting` / `context.compacted` event types to `crates/core/src/events.rs`
2. Emit events from `ReasonAtom` during compaction flow
3. Add `LlmCompactionInfo` to UI TypeScript types
4. Render compaction divider in chat UI
5. Render Infinity Context trim notice in chat UI

### Phase 2: Observation Masking
1. Implement tool output masking in `crates/core/src/message_filter.rs`
2. Apply masking before compaction in `ReasonAtom`
3. Add masking as a strategy in compaction events

### Phase 3: Proactive Compaction
1. Add token estimation to message assembly in `ReasonAtom`
2. Trigger compaction at 85% budget threshold
3. Wire up observation masking → compaction → summarization cascade

### Phase 4: Provider-Agnostic Summarization
1. Implement LLM-based conversation summarization
2. Use as fallback when driver doesn't support `/responses/compact`
3. Add summary quality evaluation to existing eval framework

## Open Questions

1. **Should compaction be undoable?** If a user notices the agent lost context after compaction, can they "expand" back? Expensive to implement, but improves trust.
2. **Compaction during streaming?** If the model is mid-stream and we detect approaching limits, do we interrupt and compact, or finish the current turn first?
3. **Cross-session compaction?** Should compaction summaries persist across session restarts, or start fresh?
4. **Compaction instructions?** Claude Code allows `/compact Focus on API changes`. Should we support user-directed compaction focus?

## References

- [Anthropic: Effective Context Engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [Anthropic: Compaction API](https://platform.claude.com/docs/en/build-with-claude/compaction)
- [JetBrains: The Complexity Trap (NeurIPS 2025)](https://arxiv.org/abs/2508.21433) — observation masking vs summarization
- [Microsoft: LLMLingua](https://llmlingua.com/llmlingua.html) — token-level compression
- [NAACL 2025: Prompt Compression Survey](https://aclanthology.org/2025.naacl-long.368.pdf)
- [OpenAI: GPT-5.2 Codex with Context Compaction](https://openai.com/index/introducing-gpt-5-2/)
- [Google ADK: Context Compaction](https://google.github.io/adk-docs/context/compaction/)
- `specs/infinity-context.md` — existing Infinity Context spec
- `specs/events.md` — event schema
