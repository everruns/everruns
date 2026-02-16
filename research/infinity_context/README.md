# Infinity Context Evaluation Framework

Research evaluation framework for comparing context management strategies in long conversations.

## Overview

Compares three approaches to handling conversations that exceed LLM context limits:

1. **Baseline**: Sends all messages. Fails when context exceeded.
2. **Naive Trim**: Drops oldest messages. Simple but loses information.
3. **Infinity Context**: Trims messages but provides `query_history` tool for retrieving excluded history.

## Usage

```bash
# Generate dataset
cargo run -- generate -o datasets/infinity_context.jsonl

# Run evaluation (with delays to avoid rate limits)
cargo run -- run -d datasets/infinity_context.jsonl \
  --delay-ms 5000 \
  --inter-call-delay-ms 10000

# Run specific scenario/capability
cargo run -- run -d datasets/infinity_context.jsonl \
  --capability "Infinity Context" \
  --scenario needle_basic_0

# Dry run (no LLM calls)
cargo run -- run -d datasets/infinity_context.jsonl --dry-run
```

## Scenarios

Generated scenarios test each strategy:

- **Needle in Haystack**: Important info early, retrieve later
- **Multi-hop Reasoning**: Connect facts across conversation
- **Cumulative Tasks**: Results build on each other
- **Final Decision**: Early context affects final answer
- **Decision Timeline**: Track decision changes over time
- **Tool Result Disambiguation**: Distinguish similar tool outputs

## Metrics

- **Score**: LLM-as-judge evaluation (0-100%)
- **Token Usage**: Input/output tokens
- **Latency**: Time to produce response
- **History Queries**: Times `query_history` was called

## Project Structure

```
research/infinity_context/
├── src/
│   ├── main.rs              # CLI
│   ├── runner.rs            # Evaluation orchestration
│   ├── dataset.rs           # Dataset loading
│   ├── scenarios/           # Scenario generation
│   ├── capabilities/        # Strategy implementations
│   ├── scorer.rs            # LLM-as-judge scoring
│   ├── metrics.rs           # Metric aggregation
│   └── report.rs            # Markdown reports
├── datasets/                # Generated datasets (gitignored)
└── results/                 # Evaluation reports (gitignored)
```

## See Also

- [Infinity Context Spec](../../specs/infinity-context.md)
- [Capabilities Spec](../../specs/capabilities.md)
