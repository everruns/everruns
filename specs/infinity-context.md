# Infinity Context

## Abstract

Current LLM context windows (128k-200k tokens) impose hard limits on agent conversations. When exceeded, systems either fail with errors or naively truncate older messages, losing critical context. This spec defines an "infinity context" capability that enables agents to work with conversations of unlimited length by:

1. Sending only recent messages to the LLM (within budget)
2. Providing a system message indicating additional history exists
3. Giving the LLM a tool to query/search historical messages on-demand

This approach lets the model "pull" relevant context rather than us "pushing" everything, enabling arbitrarily long conversations while preserving access to all historical information.

**Relationship to Compaction:** Compaction (`specs/compaction.md`) actively reduces message size by stripping reproducible content or summarizing. Infinity Context manages what to send when history is too large. They are complementary and independent — compaction reduces tokens per message, infinity context manages which messages to include.

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
    "min_recent_messages": 10
  }
  ```

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

```python
def select_messages(all_messages, budget_tokens, min_recent):
    # Always include system message
    selected = [system_message]
    budget -= estimate_tokens(system_message)

    # Always include min_recent most recent messages
    recent = all_messages[-min_recent:]
    for msg in recent:
        selected.append(msg)
        budget -= estimate_tokens(msg)

    # Add older messages while budget allows (newest first)
    remaining = all_messages[:-min_recent]
    for msg in reversed(remaining):
        msg_tokens = estimate_tokens(msg)
        if budget - msg_tokens < 0:
            break
        selected.insert(1, msg)  # After system, before recent
        budget -= msg_tokens

    excluded = [m for m in all_messages if m not in selected]
    return selected, excluded
```

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
