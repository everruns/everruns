# Phoenix Observability Specification

## Abstract

Arize Phoenix provides LLM-specific observability for Everruns, enabling developers to trace, debug, and analyze LLM interactions. Phoenix is integrated via OpenTelemetry OTLP protocol and supports both OpenTelemetry gen-ai semantic conventions and OpenInference conventions for enhanced visualization.

## Architecture

```mermaid
graph TD
    subgraph Everruns
        CP[Control-Plane]
        Worker[Worker]
        OTel[OtelEventListener]
    end

    subgraph Events
        LLM[llm.generation]
        Tool[tool.call_completed]
        Turn[turn.completed]
    end

    subgraph Phoenix
        OTLP[OTLP Receiver :4317]
        UI[Phoenix UI :6006]
        Store[Trace Storage]
    end

    CP --> OTel
    Worker --> OTel
    OTel --> LLM
    OTel --> Tool
    OTel --> Turn
    LLM --> OTLP
    Tool --> OTLP
    Turn --> OTLP
    OTLP --> Store
    Store --> UI
```

## Requirements

### Deployment Options

1. **Local Development (Default)**:
   - Phoenix runs in Docker via `docker-compose.yml` with `phoenix` profile
   - `just start-all` starts Phoenix automatically
   - UI accessible at `http://localhost:6006`
   - OTLP endpoint at `localhost:4317`

2. **Self-Hosted Production**:
   - Deploy Phoenix container to Kubernetes or Docker Swarm
   - Configure `OTEL_EXPORTER_OTLP_ENDPOINT` to point to Phoenix

3. **Arize Cloud**:
   - Use Arize's managed Phoenix service
   - Configure endpoint and API key via environment variables

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Phoenix OTLP endpoint | `http://localhost:4317` |
| `OTEL_EXPORTER_OTLP_HEADERS` | Headers for authentication (Arize Cloud) | - |
| `OTEL_SDK_DISABLED` | Disable tracing entirely | `false` |
| `OTEL_SERVICE_NAME` | Service name in traces | `everruns-*` |
| `OTEL_RECORD_CONTENT` | Record LLM input/output content | `false` |

### Semantic Conventions

Everruns emits both OpenTelemetry gen-ai and OpenInference attributes for maximum compatibility.

#### OpenTelemetry Gen-AI Attributes

| Attribute | Description |
|-----------|-------------|
| `gen_ai.operation.name` | Operation type (`chat`, `execute_tool`, `invoke_agent`) |
| `gen_ai.system` | Provider name (`openai`, `anthropic`) |
| `gen_ai.request.model` | Model identifier |
| `gen_ai.usage.input_tokens` | Prompt tokens used |
| `gen_ai.usage.output_tokens` | Completion tokens used |
| `gen_ai.response.finish_reasons` | Completion stop reason |
| `gen_ai.conversation.id` | Session ID for correlation |

#### OpenInference Attributes (Phoenix-Specific)

| Attribute | Description |
|-----------|-------------|
| `openinference.span.kind` | Span type (`LLM`, `TOOL`, `AGENT`) |
| `llm.model_name` | Model identifier |
| `llm.system` | AI product identifier |
| `llm.token_count.prompt` | Input tokens |
| `llm.token_count.completion` | Output tokens |
| `llm.token_count.total` | Total tokens |
| `session.id` | Session identifier |
| `tool.name` | Tool being executed |

### Span Hierarchy

Spans are hierarchical to show the full agentic loop:

```
AGENT span (turn.started → turn.completed)
├── LLM span (llm.generation)
├── TOOL span (tool.call_started → tool.call_completed)
├── LLM span (llm.generation)
└── ...
```

This hierarchy enables Phoenix to visualize:
- The full agent iteration loop with all LLM calls and tool executions
- Token usage aggregated per turn
- Tool calls in context of the agent's reasoning
- Duration of each step relative to the overall turn

### Span Types

1. **AGENT Spans** (`openinference.span.kind=AGENT`):
   - **Root span** for each conversation turn
   - Opened at `turn.started`, closed at `turn.completed`
   - Children: LLM spans and TOOL spans
   - Includes aggregated token usage, iteration count, total duration
   - Name format: `agent_turn {turn_id}`

2. **LLM Spans** (`openinference.span.kind=LLM`):
   - **Child of AGENT span**
   - Created for each `llm.generation` event
   - Point-in-time span (represents completed generation)
   - Includes token counts, latency, model info, finish reason
   - Name format: `chat {model}`

3. **TOOL Spans** (`openinference.span.kind=TOOL`):
   - **Child of AGENT span**
   - Opened at `tool.call_started`, closed at `tool.call_completed`
   - Includes tool name, success status, execution duration
   - Name format: `tool {name}`

### Docker Configuration

Phoenix service in `local/docker-compose.yml`:

```yaml
phoenix:
  image: arizephoenix/phoenix:latest
  container_name: everruns-phoenix
  profiles:
    - phoenix
  ports:
    - "6006:6006"    # Phoenix UI
    - "4317:4317"    # OTLP gRPC
    - "4318:4318"    # OTLP HTTP
  environment:
    PHOENIX_ENABLE_PROMETHEUS: "false"
    PHOENIX_WORKING_DIR: /phoenix_data
  volumes:
    - phoenix_data:/phoenix_data
```

### Just Commands

| Command | Description |
|---------|-------------|
| `just start-all` | Start all services with Phoenix (default) |
| `just start-phoenix` | Start Docker with Phoenix profile only |
| `just start-docker` | Start Docker with Jaeger profile |
| `just stop-docker` | Stop all Docker services |

### Switching Between Backends

Phoenix and Jaeger use the same OTLP ports. To switch:

```bash
# Stop current backend
just stop-docker

# Start with Phoenix (default for start-all)
just start-phoenix

# Or start with Jaeger
just start-docker
```

## Implementation Details

### OtelEventListener

Location: `crates/core/src/observation/otel.rs`

The listener maintains **hierarchical spans** across event boundaries:

```rust
pub struct OtelEventListener {
    /// Active turns per session (only one turn active per session)
    /// Key: session_id, Value: active turn info with span
    active_turns: Mutex<HashMap<Uuid, ActiveTurn>>,

    /// Active tool calls (can have multiple concurrent)
    /// Key: tool_call_id, Value: active tool call info with span
    active_tool_calls: Mutex<HashMap<String, ActiveToolCall>>,
}

impl EventListener for OtelEventListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            // AGENT span lifecycle
            EventData::TurnStarted(data) => self.handle_turn_started(event, data),
            EventData::TurnCompleted(data) => self.handle_turn_completed(event, data),
            // LLM span (child of AGENT)
            EventData::LlmGeneration(data) => self.handle_llm_generation(event, data),
            // TOOL span lifecycle (child of AGENT)
            EventData::ToolCallStarted(data) => self.handle_tool_call_started(event, data),
            EventData::ToolCallCompleted(data) => self.handle_tool_call_completed(event, data),
            _ => {}
        }
    }
}
```

**Key implementation details:**

1. **AGENT spans are kept open** from `turn.started` until `turn.completed`
2. **LLM spans are created as children** using `parent.in_scope(|| create_span())`
3. **TOOL spans are kept open** from `tool.call_started` until `tool.call_completed`
4. **Session-based correlation**: Only one turn active per session, so `session_id` links child events to parent turn

### Telemetry Module

Location: `crates/core/src/telemetry.rs`

Defines semantic convention constants:

- `gen_ai::*` - OpenTelemetry gen-ai conventions
- `openinference::*` - OpenInference conventions for Phoenix

## Privacy Considerations

1. **Content Recording**: Disabled by default (`OTEL_RECORD_CONTENT=false`)
2. **Token Counts Only**: By default, only metrics are captured, not message content
3. **Session Correlation**: Uses session IDs for trace correlation without exposing user data

## Troubleshooting

### No Traces in Phoenix

1. Verify Phoenix is running: `docker ps | grep phoenix`
2. Check OTLP endpoint: `echo $OTEL_EXPORTER_OTLP_ENDPOINT`
3. Ensure `OTEL_SDK_DISABLED` is not `true`
4. Check service logs for OTLP connection errors

### Missing OpenInference Attributes

Ensure using Everruns v0.4.0+ which includes OpenInference support.

### Port Conflicts

Phoenix and Jaeger use the same ports (4317, 4318). Only run one at a time.
