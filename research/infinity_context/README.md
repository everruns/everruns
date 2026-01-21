# Infinity Context Evaluation Framework

Research evaluation framework for comparing context management strategies in long conversations.

## Overview

This evaluation framework compares three approaches to handling conversations that exceed LLM context limits:

1. **Baseline (No Trimming)**: Sends all messages to the model. Fails when context is exceeded.
2. **Naive Trim**: Drops oldest messages to fit within budget. Simple but loses information.
3. **Infinity Context**: Trims messages but provides a `query_history` tool for the LLM to retrieve excluded history on-demand.

## Running Evaluations

```bash
# From repo root
just eval              # Dry run (no LLM calls, shows token estimates)
just eval-live         # Full run with LLM calls
just eval-dry          # Explicit dry run

# Or run directly
cd research/infinity_context
cargo run --release -- --synthetic
cargo run --release -- --synthetic --dry-run
```

## Scenarios

The framework generates synthetic scenarios to test each strategy:

- **Needle in Haystack**: Important information appears early, needs to be retrieved later
- **Multi-hop Reasoning**: Requires connecting facts spread across the conversation
- **Cumulative Tasks**: Results build on each other across the conversation

## Metrics

The framework tracks:

- **Task Completion Rate**: Did the agent produce the correct answer?
- **Token Efficiency**: Tokens used vs baseline
- **Latency**: Time to produce response
- **History Queries**: Number of times the agent used `query_history` tool

## Integration with Core

The context strategy capabilities are implemented in `crates/core/src/capabilities/context_strategies.rs`:

- `NaiveTrimCapability` - Drops old messages via `BatchTransform` filter
- `InfinityContextCapability` - Provides `query_history` tool + message injection

These capabilities use the `MessageFilterProvider` trait to modify how messages are loaded for the LLM.

## Project Structure

```
research/infinity_context/
├── src/
│   ├── main.rs           # CLI entry point
│   ├── runner.rs         # Evaluation orchestration
│   ├── metrics.rs        # Metric collection
│   ├── report.rs         # Markdown report generation
│   └── strategies/       # Strategy implementations for eval
├── scenarios/            # Scenario configurations
└── results/              # Generated evaluation reports
```

## Configuration

Strategies accept these configuration options:

```json
{
  "context_budget_tokens": 100000,   // Max tokens to send to LLM
  "min_recent_messages": 10,         // Always keep this many recent messages
  "boost_recency": true,             // Prefer recent messages in search
  "boost_conversation": true         // Boost user/assistant over tool results
}
```

## See Also

- [Infinity Context Spec](../../specs/infinity-context.md) - Full specification
- [Capabilities Spec](../../specs/capabilities.md) - Agent capabilities system
