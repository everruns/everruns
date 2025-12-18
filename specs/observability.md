# Observability Specification

## Abstract

Everruns provides optional observability integration for monitoring and analyzing agent execution. The observability system is designed to be:
- **Non-intrusive**: Agent loop code has no knowledge of observability backends
- **Extensible**: New observability backends can be added without modifying core code
- **Event-driven**: Uses the existing event emission system for data collection
- **Optional**: Zero runtime cost when not configured

The initial implementation supports Langfuse as the first observability backend, with the architecture designed to support additional backends (DataDog, custom OpenTelemetry, etc.).

## Requirements

### Core Architecture

1. **Event Subscription Pattern**: Observability hooks into the `EventEmitter` trait, wrapping the existing implementation to capture events without modifying the agent loop.

2. **Crate Structure**:
   - `everruns-observability` - Observability abstractions and backends
   - Contains `ObservabilityBackend` trait for pluggable backends
   - Contains `ObservableEventEmitter` wrapper for transparent event capture

3. **Data Model**:
   - `ObservabilityEvent` - High-level events derived from `LoopEvent`
   - Traces (session/run level)
   - Generations (LLM call spans)
   - Tool executions (tool call spans)

### Configuration

1. **Environment Variables**:
   - `OBSERVABILITY_ENABLED` - Global enable/disable (default: auto-detect from backend config)
   - `LANGFUSE_PUBLIC_KEY` - Langfuse public key (pk-lf-...)
   - `LANGFUSE_SECRET_KEY` - Langfuse secret key (sk-lf-...)
   - `LANGFUSE_HOST` - Langfuse API host (default: https://cloud.langfuse.com)
   - `LANGFUSE_RELEASE` - Application release/version tag
   - `LANGFUSE_FLUSH_INTERVAL_MS` - Batch flush interval (default: 5000)
   - `LANGFUSE_MAX_BATCH_SIZE` - Max batch size before forced flush (default: 100)

2. **Feature Flags**:
   - `langfuse` feature on `everruns-observability` crate controls Langfuse dependencies

### Backend Trait

```rust
#[async_trait]
pub trait ObservabilityBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_enabled(&self) -> bool;
    async fn record(&self, event: ObservabilityEvent) -> Result<(), ObservabilityError>;
    async fn flush(&self) -> Result<(), ObservabilityError>;
    async fn shutdown(&self) -> Result<(), ObservabilityError>;
}
```

### Observability Events

Events map to the Langfuse data model:

| LoopEvent | ObservabilityEvent | Langfuse Entity |
|-----------|-------------------|-----------------|
| LoopStarted | TraceStarted | Trace |
| LoopCompleted/LoopError | TraceCompleted | Trace update |
| LlmCallStarted | GenerationStarted | Generation |
| LlmCallCompleted | GenerationCompleted | Generation update |
| ToolExecutionStarted | ToolStarted | Span |
| ToolExecutionCompleted | ToolCompleted | Span update |

### Integration Points

1. **Worker Integration**:
   - `create_observable_agent_loop()` factory creates agent loops with observability
   - Automatically configures backends from environment variables
   - Falls back to standard agent loop when not configured

2. **Batching**:
   - Events are batched for efficient network usage
   - Configurable batch size and flush interval
   - Automatic flush on shutdown

### Langfuse Backend

1. **API Integration**:
   - Uses Langfuse's `/api/public/ingestion` batch endpoint
   - Basic authentication with public/secret key
   - HTTP/JSON transport (no gRPC dependency)

2. **Data Mapping**:
   - Session ID → Langfuse session_id
   - Agent ID → Langfuse user_id
   - Trace ID → Langfuse trace ID (UUID v7)
   - Span ID → Langfuse observation ID

3. **Supported Features**:
   - Trace creation with session context
   - LLM generation tracking
   - Tool execution spans
   - Duration and latency tracking
   - Token usage (when available)

### Future Backends

The architecture supports adding new backends:

1. **OpenTelemetry Direct**: Export to any OTLP endpoint
2. **DataDog**: Native DataDog APM integration
3. **Custom Webhooks**: Send events to custom endpoints
4. **File/Console**: Debug logging for development

Each backend implements `ObservabilityBackend` and is configured via environment variables.

## Usage

### Enabling Langfuse

```bash
# Required
export LANGFUSE_PUBLIC_KEY="pk-lf-..."
export LANGFUSE_SECRET_KEY="sk-lf-..."

# Optional
export LANGFUSE_HOST="https://us.cloud.langfuse.com"
export LANGFUSE_RELEASE="v1.0.0"
```

### Disabling Observability

```bash
export OBSERVABILITY_ENABLED=false
```

Or simply don't set `LANGFUSE_PUBLIC_KEY` and `LANGFUSE_SECRET_KEY`.

## Testing

Unit tests verify:
- Event conversion logic
- Batch management
- Backend trait implementations

Integration tests require a Langfuse instance and verify end-to-end event flow.
