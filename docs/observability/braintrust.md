# Braintrust Integration

Everruns integrates with [Braintrust](https://www.braintrust.dev/) to provide LLM observability, evaluation, and trace visualization for your agentic workflows.

## What You Get

- **Trace Visualization**: See complete agent turns as hierarchical traces
- **Token Usage Tracking**: Monitor input/output tokens and prompt cache efficiency
- **Performance Metrics**: Time-to-first-token, LLM call duration, tool execution times
- **Debug Workflows**: Inspect multi-step agent reasoning with tool calls

## Quick Start

### 1. Get Your API Key

1. Sign up at [braintrust.dev](https://www.braintrust.dev/)
2. Go to **Settings** → **API Keys**
3. Create a new API key

### 2. Configure Everruns

Set environment variables:

```bash
# Required
export BRAINTRUST_API_KEY=sk-bt-your-api-key

# Recommended: specify your project name
export BRAINTRUST_PROJECT_NAME="My Project"
```

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BRAINTRUST_API_KEY` | Yes | - | API key from Braintrust settings |
| `BRAINTRUST_PROJECT_NAME` | No | `My Project` | Project name for organizing traces |
| `BRAINTRUST_PROJECT_ID` | No | - | Direct project UUID (skips name lookup) |
| `BRAINTRUST_API_URL` | No | `https://api.braintrust.dev` | API base URL |

### 3. View Traces

1. Open the Braintrust dashboard
2. Navigate to your project
3. Go to **Logs** to see incoming traces

## Trace Hierarchy

Each agent turn creates a trace with the following structure:

```
agent turn (root span)
├── reason (iteration 1)
│   └── llm.generation (gpt-4o)
├── act (iteration 1)
│   ├── tool.call (search)
│   └── tool.call (fetch)
├── reason (iteration 2)
│   └── llm.generation (gpt-4o)
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

## Troubleshooting

### Traces Not Appearing

1. **Check API key**: Verify `BRAINTRUST_API_KEY` is set correctly
2. **Check logs**: Look for "Braintrust listener initialized" at startup
3. **Project name mismatch**: If `BRAINTRUST_PROJECT_NAME` doesn't match an existing project in Braintrust, logs will be silently dropped. Verify the project name exists in your Braintrust dashboard.

## Links

- [Braintrust Documentation](https://www.braintrust.dev/docs)
- [API Reference](https://www.braintrust.dev/docs/api-reference/introduction)
- [Insert Logs API](https://www.braintrust.dev/docs/api-reference/logs/insert-project-logs-events)
