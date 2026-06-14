# Infinity Context

## Abstract

Current LLM context windows (128k-200k tokens) impose hard limits on agent conversations. When exceeded, systems either fail with errors or naively truncate older messages, losing critical context. This spec defines an "infinity context" capability that enables agents to work with conversations of unlimited length by:

1. Sending only recent messages to the LLM (within budget)
2. Providing a system message indicating additional history exists
3. Giving the LLM a tool to query/search historical messages on-demand

This approach lets the model "pull" relevant context rather than us "pushing" everything, enabling arbitrarily long conversations while preserving access to all historical information.

**Relationship to Compaction:** Compaction (`specs/compaction.md`) actively reduces what is sent by stripping reproducible tool output (observation masking) and summarizing older turns into an always-present `[CONVERSATION_SUMMARY]`. Infinity Context instead keeps a recent window and exposes evicted history through `query_history` (pull-based retrieval).

These are two answers to the same problem and they are **not** freely composable. Infinity Context evicts older messages during message loading (`post_load`), before compaction runs in the reason atom, so when both are enabled infinity context would destroy history before compaction could summarize it — leaving compaction only the recent window. Therefore:

- **Compaction is the stronger primary strategy** for long-running agents: its summary keeps the gist always present, so the model never has to know to query. Pull-based retrieval has a well-known failure mode — the model does not know what it does not know — and on its own loses the original task once the window slides.
- **Infinity Context is a backstop**, valuable mainly for its lossless `query_history` search over full storage.
- When both are enabled, infinity context detects compaction (via the derived `compaction_active` flag set during capability collection) and **defers token-budget eviction to compaction**: it anchors the task, provides `query_history`, and stops trimming, so compaction owns reduction.

## Problem Statement

### Current Behavior

1. **No trimming**: All messages sent to LLM. Works until context exceeded, then fails with `RequestTooLarge` error.
2. **Naive trimming**: Drop oldest messages. Loses context permanently. Model cannot reference earlier discussion.

### Desired Behavior

Agent can work with 1000+ message conversations, accessing any historical context on-demand via tool calls.

## Requirements

### R1: Context Budget Management

- System MUST track estimated token count of messages
- System MUST enforce a configurable context budget (e.g., 70% of model's limit)
- Messages exceeding budget MUST be excluded from direct context
- Excluded messages MUST remain queryable via history tool

### R2: History Awareness

- When messages are excluded, system MUST inject a notice: "Note: {N} earlier messages not shown. Use `query_history` tool to search."
- Notice MUST include count of excluded messages and approximate timespan

### R3: History Query Tool

The `query_history` tool MUST support:

| Parameter | Type | Description |
|-----------|------|-------------|
| `query` | string | Search term for keyword/semantic search |
| `message_range` | object | `{from: int, to: int}` - 0-indexed message positions |
| `time_range` | object | `{from: ISO8601, to: ISO8601}` - timestamp bounds |
| `message_types` | array | Filter: `["user", "assistant", "tool_result"]` |
| `limit` | int | Max messages to return (default: 20) |
| `context_lines` | int | Lines before/after match to include (default: 3) |

Tool MUST return messages with:
- Original position in conversation
- Timestamp
- Abbreviated content (configurable max length)
- Relevance score (for search queries)

### R4: Priority/Recency Weighting

When searching history:
- More recent messages SHOULD rank higher (recency decay)
- User and assistant messages SHOULD rank higher than tool outputs
- Exact keyword matches SHOULD rank higher than partial matches

### R5: Graceful Degradation

- If model exceeds budget even with trimming, system MUST handle `RequestTooLarge` gracefully
- System SHOULD retry with more aggressive trimming before failing

### R6: Capability Integration

- Infinity context MUST be implemented as a capability
- Capability MUST be opt-in per agent via configuration
- Capability config MUST support:
  ```json
  {
    "context_budget_tokens": 100000,
    "min_recent_messages": 10,
    "max_recent_messages": null,
    "keep_first_messages": 1
  }
  ```
- `keep_first_messages` (default 1) is the number of leading messages always kept
  as an anchor — the original task/goal. The anchor is **additional** to
  `max_recent_messages` (which caps only the recent tail).

## Design

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     ReasonAtom                               │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ 1. Load ALL messages from storage                    │    │
│  │ 2. Apply InfinityContextFilter                       │    │
│  │    - Estimate tokens                                 │    │
│  │    - Keep recent messages within budget              │    │
│  │    - Store excluded messages in FilterContext        │    │
│  │ 3. Inject history notice if messages excluded        │    │
│  │ 4. Register query_history tool                       │    │
│  │ 5. Send to LLM                                       │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                 query_history Tool                           │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ 1. Receive query parameters                          │    │
│  │ 2. Search excluded messages (FilterContext)          │    │
│  │ 3. Apply ranking (recency, type, relevance)          │    │
│  │ 4. Return formatted results with context             │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

### Token Estimation

Use character-based approximation: `tokens ≈ chars / 4`

More accurate estimation can use tiktoken for OpenAI or anthropic-tokenizer for Claude, but character-based is sufficient for budgeting.

### Message Selection Algorithm

Selection is "protect the head + tail, drop the middle" (see
`anchored_window` in `crates/core/src/message_filter.rs`). The agent system
prompt is assembled separately and is never part of this list, so the head
anchor here protects the **first conversation message — the original task/goal**.

```python
def select_messages(all_messages, budget, keep_head, min_tail, max_tail=None):
    n = len(all_messages)
    # Always keep the first `keep_head` (the task) and last `min_tail` (recent),
    # even if they exceed budget. A hard `max_tail` cap bounds the tail.
    if max_tail is not None:
        min_tail = min(min_tail, max(max_tail, 1))
    recent_start = n - min_tail
    if recent_start <= keep_head:
        return all_messages, []  # head and tail meet: nothing to drop

    # Grow the recent block backward (newest-first) while it fits the token
    # budget and the optional max_tail cap. This keeps a single contiguous tail
    # so tool-call/result adjacency is preserved.
    cost = sum_tokens(all_messages[:keep_head]) + sum_tokens(all_messages[recent_start:])
    tail = min_tail
    while recent_start > keep_head and (max_tail is None or tail < max_tail):
        c = estimate_tokens(all_messages[recent_start - 1])
        if cost + c > budget:
            break
        recent_start -= 1; cost += c; tail += 1

    kept = all_messages[:keep_head] + all_messages[recent_start:]
    excluded = all_messages[keep_head:recent_start]   # the dropped middle
    return kept, excluded
```

The notice is inserted between the anchor and the recent block, so the live
prompt reads `[task anchor] -> [N earlier messages hidden] -> [recent window]`.

`max_recent_messages` is a hard cap for constrained surfaces such as public
support chat; it bounds only the recent tail (the head anchor is additional).
Message limits always mean the latest N messages, returned in chronological
order; older excluded messages remain available through `query_history`.

**Known limitation:** the anchor is guaranteed only within the candidate load
window (`resolve_candidate_load_limit`). For conversations longer than that
window — or when `max_recent_messages` is set very low — the DB loads only the
latest N messages, so the true first message is never fetched and the anchor
degrades to the oldest loaded message. Guaranteeing the head for arbitrarily
long histories needs a head+tail DB fetch (tracked as a follow-up).

### History Notice Format

```
[Context Notice: This conversation has 247 messages spanning 3 hours.
The 198 oldest messages are not shown to fit context limits.
Use the `query_history` tool to search or retrieve earlier messages.]
```

### Query History Tool Schema

```json
{
  "name": "query_history",
  "description": "Search or retrieve messages from earlier in this conversation that are not currently visible. Use this when you need to reference something discussed previously.",
  "parameters": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Search term to find relevant messages"
      },
      "message_range": {
        "type": "object",
        "properties": {
          "from": {"type": "integer", "description": "Start index (0-based)"},
          "to": {"type": "integer", "description": "End index (exclusive)"}
        },
        "description": "Retrieve messages by position range"
      },
      "time_range": {
        "type": "object",
        "properties": {
          "from": {"type": "string", "format": "date-time"},
          "to": {"type": "string", "format": "date-time"}
        },
        "description": "Retrieve messages within time window"
      },
      "message_types": {
        "type": "array",
        "items": {"type": "string", "enum": ["user", "assistant", "tool_result"]},
        "description": "Filter by message type"
      },
      "limit": {
        "type": "integer",
        "default": 20,
        "description": "Maximum messages to return"
      }
    }
  }
}
```

## References

- [LongBench v2](https://github.com/THUDM/LongBench) - Long context benchmark
- [L-Eval](https://github.com/OpenLMLab/LEval) - ACL'24 evaluation suite
- [Anthropic Context Caching](https://docs.anthropic.com/en/docs/build-with-claude/context-caching) - Related capability for context reuse
