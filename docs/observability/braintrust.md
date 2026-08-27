---
title: Braintrust
description: LLM observability, evaluation, and trace visualization with Braintrust. Monitor agent performance, inspect tool call chains, and compare model outputs over time.
---

<img src="/images/observability/braintrust-logo.png" alt="Braintrust" width="64" style="float: right; margin-left: 16px;" />

:::note[Preview]
This integration is in preview. APIs and behavior may change.
:::

Everruns integrates with [Braintrust](https://www.braintrust.dev/) to provide LLM observability, evaluation, and trace visualization for your agentic workflows.

## What You Get

- **Turn Traces Grouped by Session**: Keep one trace per turn while grouping the conversation by `metadata.session_id`
- **Token Usage Tracking**: Monitor input/output tokens and prompt cache efficiency
- **Performance Metrics**: Time-to-first-token, LLM call duration, tool execution times
- **Durable-ish Delivery**: Buffered batch delivery with retries for rate limits, `5xx`, and timeout/connect failures
- **Privacy Controls**: Raw content, thinking, tool args, and tool results are independently configurable

## Quick Start

### 1. Get Your API Key

1. Sign up at [braintrust.dev](https://www.braintrust.dev/)
2. Go to **Settings** → **API Keys**
3. Create a new API key

### 2. Configure Everruns

Set environment variables:

```bash
# Optional explicit switch
export BRAINTRUST_ENABLED=true

# Required
export BRAINTRUST_API_KEY=sk-bt-your-api-key

# Recommended: specify your project name
export BRAINTRUST_PROJECT_NAME="My Project"

# Conservative defaults
export BRAINTRUST_RECORD_CONTENT=false
export BRAINTRUST_RECORD_THINKING=none
export BRAINTRUST_TOOL_ARGS_MODE=redacted
export BRAINTRUST_TOOL_RESULTS_MODE=summary
```

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BRAINTRUST_ENABLED` | No | enabled when API key is present | Explicit Braintrust on/off switch |
| `BRAINTRUST_API_KEY` | Yes | - | API key from Braintrust settings |
| `BRAINTRUST_PROJECT_NAME` | No | `My Project` | Project name for organizing traces |
| `BRAINTRUST_PROJECT_ID` | No | - | Direct project UUID (skips name lookup) |
| `BRAINTRUST_API_URL` | No | `https://api.braintrust.dev` | API base URL |
| `BRAINTRUST_QUEUE_CAPACITY` | No | `1024` | Buffered event capacity before new exports are dropped |
| `BRAINTRUST_MAX_BATCH_SIZE` | No | `50` | Max events per Braintrust insert call |
| `BRAINTRUST_FLUSH_INTERVAL_MS` | No | `500` | Max delay before a partial batch flushes |
| `BRAINTRUST_REQUEST_TIMEOUT_MS` | No | `10000` | Per-request timeout |
| `BRAINTRUST_MAX_RETRIES` | No | `3` | Retries for `429`, `5xx`, and timeout/connect failures |
| `BRAINTRUST_RETRY_BASE_DELAY_MS` | No | `250` | Initial retry backoff |
| `BRAINTRUST_RETRY_MAX_DELAY_MS` | No | `5000` | Retry backoff cap |
| `BRAINTRUST_RECORD_CONTENT` | No | `false` | Export raw turn and LLM text content |
| `BRAINTRUST_RECORD_THINKING` | No | `none` | Export thinking as `none`, `summary`, or `full` |
| `BRAINTRUST_TOOL_ARGS_MODE` | No | `redacted` | Export tool args as `full`, `redacted`, or `none` |
| `BRAINTRUST_TOOL_RESULTS_MODE` | No | `summary` | Export tool results as `full`, `summary`, `redacted`, or `none` |
| `BRAINTRUST_DEBUG_PAYLOADS` | No | `false` | Print full outbound Braintrust payload JSON to local debug logs |

### 3. View Traces

1. Open the Braintrust dashboard
2. Navigate to your project
3. Go to **Logs**
4. Group or filter by `metadata.session_id` to reconstruct the full session timeline across turn traces

## Trace Hierarchy

Each Everruns turn creates its own trace with the following structure:

```
agent turn (root span)
├── reason (iteration 1)
│   └── llm.generation (gpt-5.2)
├── act (iteration 1)
│   ├── tool.call (search)
│   └── tool.call (fetch)
├── reason (iteration 2)
│   └── llm.generation (gpt-5.2)
└── (no more tool calls - turn complete)
```

### Span Types

| Span | Type | Description |
|------|------|-------------|
| Agent Turn | `task` | Root span for the entire user request |
| Reason | `task` | LLM reasoning phase (may iterate) |
| Act | `task` | Tool execution phase |
| LLM Generation | `llm` | Individual LLM API call |
| Tool Call | `tool` | Individual tool execution |

## Session Grouping

Everruns does not export one giant trace for the whole conversation.

- Each turn remains its own Braintrust trace.
- Every root turn span carries `metadata.session_id`.
- Session lifecycle events (`session.started`, `session.activated`, `session.idled`) are exported as lightweight logs with the same `session_id`.
- Root turn metadata also carries stable filtering fields when available, such as `input_message_id`, monotonic event ordering, deployment grade, session status, model/provider summary, retry info, and compaction info.

Use Braintrust grouping, timeline, or thread views on `metadata.session_id` to analyze the session as a whole while keeping per-turn debugging sharp.

## Metrics Captured

### LLM Generations

- `prompt_tokens` - Input token count
- `completion_tokens` - Output token count
- `cache_read_tokens` - Tokens read from prompt cache (Claude)
- `cache_creation_tokens` - Tokens written to prompt cache (Claude)
- `time_to_first_token` - Time until first token received
- `duration_ms` - Total LLM call duration

### Tool Calls

- `status` - Success/failure
- `duration_ms` - Execution time
- `error` - Error message (on failure)

## Delivery Behavior

- Exports enqueue into a bounded in-memory buffer.
- The exporter flushes batches to `POST /v1/project_logs/{project_id}/insert`.
- `429`, `5xx`, timeout, and connect failures are retried with jittered backoff.
- If the queue fills, new events are dropped and the exporter logs the drop counter.

This is best-effort durability, not a disk-backed queue.

## Privacy Controls

The Braintrust exporter defaults to conservative content handling:

- raw turn and LLM text are off unless `BRAINTRUST_RECORD_CONTENT=true`
- when raw content is off, the exporter emits structural metadata only; it does not emit truncated prompt/completion previews
- extended thinking is off unless `BRAINTRUST_RECORD_THINKING` says otherwise
- tool arguments default to `redacted`
- tool results default to `summary`
- tool arg/result modes still apply inside recorded LLM input/output payloads
- full outbound payload logging is off unless `BRAINTRUST_DEBUG_PAYLOADS=true`

## Troubleshooting

### Traces Not Appearing

1. **Check API key**: Verify `BRAINTRUST_API_KEY` is set correctly
2. **Check project resolution**: If `BRAINTRUST_PROJECT_NAME` does not match an existing project, startup logs will show a project resolution failure
3. **Check exporter logs**: Look for rate-limit retries, timeout retries, queue drops, or permanent insert failures

### Session Views Are Fragmented

1. Confirm root turn spans include `metadata.session_id`
2. Group Braintrust logs by `metadata.session_id`
3. Check whether privacy controls removed content you expected; the default is conservative

## Links

- [Braintrust Documentation](https://www.braintrust.dev/docs)
- [API Reference](https://www.braintrust.dev/docs/api-reference/introduction)
- [Insert Logs API](https://www.braintrust.dev/docs/api-reference/logs/insert-project-logs-events)
