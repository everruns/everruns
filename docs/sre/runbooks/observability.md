# Observability Configuration

This runbook describes how to configure observability integrations for Everruns.

## Overview

Everruns supports optional observability integration to monitor agent execution, LLM calls, and tool usage. The observability system captures:

- **Traces**: Session-level execution context
- **Generations**: LLM call details (model, tokens, latency)
- **Spans**: Tool execution details (name, duration, success/failure)

## Supported Backends

### Langfuse

[Langfuse](https://langfuse.com) is an open-source LLM observability platform that provides:
- Trace visualization
- Cost tracking
- Performance analytics
- Prompt management

## Configuration

### Langfuse Setup

1. **Create a Langfuse account** at https://cloud.langfuse.com (or self-host)

2. **Create a project** and obtain API keys from Settings > API Keys

3. **Configure environment variables**:

```bash
# Required
LANGFUSE_PUBLIC_KEY=pk-lf-your-public-key
LANGFUSE_SECRET_KEY=sk-lf-your-secret-key

# Optional
LANGFUSE_HOST=https://cloud.langfuse.com  # Default, or your self-hosted URL
LANGFUSE_RELEASE=v1.0.0                    # Application version tag
LANGFUSE_FLUSH_INTERVAL_MS=5000            # Batch flush interval (default: 5000)
LANGFUSE_MAX_BATCH_SIZE=100                # Max events per batch (default: 100)
```

4. **Restart Everruns** to apply the configuration

### Disabling Observability

To disable observability even when backend credentials are configured:

```bash
OBSERVABILITY_ENABLED=false
```

## Verification

After configuration, verify observability is working:

1. **Check API logs** for initialization message:
   ```
   INFO Langfuse observability backend initialized
   ```

2. **Execute an agent session** through the UI or API

3. **View traces in Langfuse**:
   - Navigate to your Langfuse project
   - Open the Traces view
   - You should see a trace for your session with:
     - LLM generations
     - Tool execution spans (if applicable)

## Troubleshooting

### No traces appearing

1. **Verify API keys** are correct and have write permissions
2. **Check network connectivity** to Langfuse host
3. **Review API logs** for error messages:
   ```
   WARN Failed to initialize Langfuse backend: ...
   ```

### Missing data in traces

The following data is captured automatically:
- Session ID as Langfuse session_id
- Agent ID as Langfuse user_id
- Iteration count in trace metadata
- Tool execution success/failure status

Token usage is captured when provided by the LLM provider.

### High latency

Observability events are batched and sent asynchronously. If you notice delays:

1. **Reduce batch size** for faster flushing:
   ```bash
   LANGFUSE_MAX_BATCH_SIZE=10
   ```

2. **Reduce flush interval**:
   ```bash
   LANGFUSE_FLUSH_INTERVAL_MS=1000
   ```

## Self-Hosted Langfuse

For self-hosted Langfuse deployments:

1. **Deploy Langfuse** following the [self-hosting guide](https://langfuse.com/docs/deployment/self-host)

2. **Configure the custom host**:
   ```bash
   LANGFUSE_HOST=https://your-langfuse-instance.com
   ```

3. **Ensure TLS** is properly configured if using HTTPS

## Future Backends

Additional observability backends (OpenTelemetry, DataDog, etc.) may be added in future releases. The architecture supports multiple concurrent backends.

## Related Documentation

- [Environment Variables](../environment-variables.md)
- [Observability Specification](../../../specs/observability.md)
