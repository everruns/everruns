# Infinity Context

## Abstract

Current LLM context windows (128k-200k tokens) impose hard limits on agent conversations. When exceeded, systems either fail with errors or naively truncate older messages, losing critical context. This spec defines an "infinity context" capability that enables agents to work with conversations of unlimited length by:

1. Sending only recent messages to the LLM (within budget)
2. Providing a system message indicating additional history exists
3. Giving the LLM a tool to query/search historical messages on-demand

This approach lets the model "pull" relevant context rather than us "pushing" everything, enabling arbitrarily long conversations while preserving access to all historical information.

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

## Evaluation Framework

### Purpose

Validate that infinity context improves agent performance on long conversations compared to:
1. **Baseline**: No trimming (fails on long conversations)
2. **Naive trim**: Drop oldest messages (loses context)
3. **Infinity context**: Trim + history tool (proposed solution)

### Evaluation Dimensions

| Dimension | Metric | Description |
|-----------|--------|-------------|
| Task completion | accuracy | Did agent complete the task correctly? |
| Context retrieval | recall@k | Did agent find relevant historical info when needed? |
| Token efficiency | total_tokens | Total tokens used across all LLM calls |
| Latency | time_to_complete | Wall clock time to complete task |
| Tool usage | query_count | Number of history queries made |

### Test Scenarios

#### Scenario Type 1: Needle in Haystack (Historical Reference)

Task requires information from early in a long conversation.

```yaml
scenario: needle_historical
setup:
  - Generate 100 filler messages (code discussion)
  - At message 15, user mentions "the API key is abc123"
  - Generate 100 more filler messages
task: "What API key did we discuss earlier?"
expected: "abc123"
measures: [accuracy, query_count]
```

#### Scenario Type 2: Multi-Hop Reference

Task requires synthesizing information from multiple earlier points.

```yaml
scenario: multi_hop
setup:
  - Message 10: "Let's call the service UserAuth"
  - Message 50: "UserAuth should use JWT tokens"
  - Message 100: "The JWT secret is in .env"
  - 150 filler messages
task: "How is the UserAuth service configured for authentication?"
expected: Contains "JWT" and ".env"
measures: [accuracy, query_count]
```

#### Scenario Type 3: Cumulative Task

Task requires remembering incremental changes throughout conversation.

```yaml
scenario: cumulative_changes
setup:
  - Iteratively build a function across 50 messages
  - Each message adds/modifies a feature
task: "Show the final version of the function we built"
expected: Contains all accumulated changes
measures: [accuracy, completeness_score]
```

### Dataset Sources

1. **Synthetic scenarios**: Generated conversations with planted information
2. **LongBench v2**: Real long-context QA tasks (HuggingFace: THUDM/LongBench-v2)
3. **L-Eval**: Long document understanding tasks (HuggingFace: L4NLP/LEval)
4. **Real conversations**: Anonymized agent session logs (future)

### Evaluation Harness Design

```
evals/
├── Cargo.toml              # Standalone binary crate
├── src/
│   ├── main.rs             # CLI entry point
│   ├── strategies/
│   │   ├── mod.rs
│   │   ├── baseline.rs     # No trimming
│   │   ├── naive_trim.rs   # Drop oldest
│   │   └── infinity.rs     # Proposed solution
│   ├── scenarios/
│   │   ├── mod.rs
│   │   ├── loader.rs       # Load from YAML/JSON
│   │   ├── needle.rs       # Needle-in-haystack generator
│   │   └── synthetic.rs    # Synthetic conversation generator
│   ├── runner.rs           # Execute scenarios against strategies
│   ├── metrics.rs          # Compute and aggregate metrics
│   └── report.rs           # Generate markdown report
├── scenarios/              # YAML scenario definitions
│   ├── needle_basic.yaml
│   ├── multi_hop.yaml
│   └── cumulative.yaml
└── results/                # Generated reports
    └── .gitkeep
```

### Running Evaluations

```bash
# Run all scenarios with all strategies
just eval

# Run specific scenario
just eval --scenario needle_basic

# Run with specific strategy
just eval --strategy infinity

# Generate comparison report
just eval --report
```

### Report Format

```markdown
# Infinity Context Evaluation Report

Generated: 2025-01-21T10:00:00Z
Model: claude-sonnet-4-20250514
Scenarios: 15

## Summary

| Strategy | Accuracy | Avg Tokens | Avg Latency | Failures |
|----------|----------|------------|-------------|----------|
| baseline | 45% | 150,000 | 12.3s | 8 (context exceeded) |
| naive_trim | 62% | 45,000 | 4.1s | 0 |
| infinity | 89% | 52,000 | 5.2s | 0 |

## Per-Scenario Results

### needle_basic
...

## Analysis

The infinity context strategy shows 27% improvement over naive trimming
while using only 15% more tokens. Key findings:
- Baseline fails on 53% of scenarios due to context limits
- Naive trim loses critical context in multi-hop scenarios
- Infinity context successfully retrieves historical info via tool
```

## Implementation Plan

### Phase 1: Evaluation Framework (This PR)
1. Create `evals/` crate structure
2. Implement strategy traits and baseline strategies
3. Build synthetic scenario generator
4. Create runner and basic reporting

### Phase 2: Infinity Context Capability
1. Implement token estimation
2. Create message selection algorithm
3. Build `query_history` tool
4. Integrate as capability

### Phase 3: Production Hardening
1. Add semantic search (optional)
2. Optimize token estimation (tiktoken)
3. Add conversation summarization (future enhancement)
4. Production metrics and monitoring

## Open Questions

1. **Semantic vs keyword search**: Should `query_history` support semantic search? Adds complexity but improves relevance.

2. **Context budget**: 70% is arbitrary. Should we tune this? Too high risks overflow, too low wastes context.

3. **Summary injection**: Should we summarize excluded messages instead of just noting their existence? Adds tokens but provides passive context.

4. **Tool result handling**: Tool outputs are often large. Should we aggressively trim these vs user/assistant messages?

## References

- [LongBench v2](https://github.com/THUDM/LongBench) - Long context benchmark
- [L-Eval](https://github.com/OpenLMLab/LEval) - ACL'24 evaluation suite
- [Anthropic Context Caching](https://docs.anthropic.com/en/docs/build-with-claude/context-caching) - Related capability for context reuse
