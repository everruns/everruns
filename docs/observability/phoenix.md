---
title: Arize Phoenix
description: LLM observability with Arize Phoenix for tracing, debugging, and analyzing AI interactions
---

Arize Phoenix provides specialized observability for LLM applications, offering insights into token usage, latency, costs, and conversation flows that generic tracing tools like Jaeger don't provide.

## Quick Start

Phoenix is the default tracing backend when running Everruns locally:

```bash
# Start all services with Phoenix
just start-all

# Phoenix UI available at
open http://localhost:6006
```

## Features

### Token Usage Analysis

Phoenix visualizes token consumption across your LLM calls:

- **Input tokens** - Prompt size for each request
- **Output tokens** - Completion size for each response
- **Total tokens** - Aggregated usage per session/turn

### Latency Metrics

Track performance metrics for your AI interactions:

- **Time to first token** - Streaming response latency
- **Total duration** - Complete request-response time
- **Tool execution time** - Duration of each tool call

### Conversation View

Phoenix displays full conversation context:

- Message history for each trace
- Tool calls and their results
- Multi-turn conversation flows

### Cost Tracking

Monitor LLM API costs when using providers that report pricing.

## Trace Types

Everruns emits three types of traces to Phoenix:

### LLM Traces

Created for each LLM API call with:
- Model name and provider
- Token counts (input, output, total)
- Latency metrics
- Finish reason

### Tool Traces

Created when tools are executed:
- Tool name
- Execution duration
- Success/failure status

### Agent Traces

Created for each conversation turn:
- Iteration count
- Aggregated token usage
- Total turn duration

## Configuration

### Local Development

Phoenix runs automatically with `just start-all`. Configuration options:

```bash
# Enable content recording (disabled by default for privacy)
export OTEL_RECORD_CONTENT=true

# Custom service name
export OTEL_SERVICE_NAME=my-everruns
```

### Production Deployment

#### Self-Hosted Phoenix

Deploy Phoenix to your infrastructure and configure the endpoint:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://phoenix.internal:4317
```

#### Arize Cloud

Use Arize's managed Phoenix service:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=https://otlp.arize.com
export OTEL_EXPORTER_OTLP_HEADERS="x-api-key=your-arize-api-key"
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Phoenix OTLP endpoint | `http://localhost:4317` |
| `OTEL_EXPORTER_OTLP_HEADERS` | Headers (for Arize Cloud auth) | - |
| `OTEL_SDK_DISABLED` | Disable tracing | `false` |
| `OTEL_SERVICE_NAME` | Service name in traces | `everruns-*` |
| `OTEL_RECORD_CONTENT` | Record message content | `false` |

## Alternative: Jaeger

If you prefer Jaeger for general-purpose tracing:

```bash
# Start with Jaeger instead of Phoenix
just start-docker

# Jaeger UI at
open http://localhost:16686
```

Note: Phoenix and Jaeger use the same ports. Only run one at a time.

## Troubleshooting

### No Traces Appearing

1. Verify Phoenix is running:
   ```bash
   docker ps | grep phoenix
   ```

2. Check the OTLP endpoint is configured:
   ```bash
   echo $OTEL_EXPORTER_OTLP_ENDPOINT
   ```

3. Ensure tracing is not disabled:
   ```bash
   echo $OTEL_SDK_DISABLED  # Should be empty or "false"
   ```

### Port Conflicts

If port 4317 is in use:

```bash
# Stop all Docker services
just stop-docker

# Restart with Phoenix
just start-phoenix
```

### Missing Token Counts

Token counts require the LLM provider to return usage information. Most providers (OpenAI, Anthropic) include this by default.

## Privacy Considerations

By default, Everruns only sends metrics to Phoenix, not message content:

- Token counts
- Latency metrics
- Model names
- Tool names

To enable content recording for debugging:

```bash
export OTEL_RECORD_CONTENT=true
```

**Warning**: Only enable in development. Content may include sensitive user data.
