# Temporal Integration Specification

## Abstract

This specification documents the Temporal workflow integration for agent execution in Everruns. When running in Temporal mode, agent runs are executed as durable, distributed workflows rather than in-process tasks.

## Architecture

```
┌─────────────────────┐                    ┌─────────────────────┐
│     API Process     │                    │   Worker Process    │
│                     │                    │                     │
│  ┌───────────────┐  │                    │  ┌───────────────┐  │
│  │TemporalRunner │  │                    │  │TemporalWorker │  │
│  │               │  │                    │  │               │  │
│  │ - start_run() │  │    Temporal        │  │ - Workflow    │  │
│  │   queues      │──┼───► Server ◄───────┼──│   Poller      │  │
│  │   workflow    │  │                    │  │ - Activity    │  │
│  │               │  │                    │  │   Poller      │  │
│  └───────────────┘  │                    │  └───────────────┘  │
│                     │                    │                     │
│  ┌───────────────┐  │                    │  ┌───────────────┐  │
│  │ SSE Endpoint  │◄─┼────────────────────┼──│  Activities   │  │
│  │ (polls DB)    │  │                    │  │ (persist to   │  │
│  └───────────────┘  │                    │  │  database)    │  │
│                     │                    │  └───────────────┘  │
└─────────────────────┘                    └─────────────────────┘
         │                                           │
         │              ┌───────────┐                │
         └──────────────│ PostgreSQL │───────────────┘
                        │ (events,   │
                        │  messages) │
                        └───────────┘
```

## Components

### TemporalClient (`temporal_client.rs`)

Client wrapper for starting workflows from the API process.

```rust
pub struct TemporalClient {
    gateway: Arc<ServerGateway>,
    config: RunnerConfig,
}

impl TemporalClient {
    async fn start_agent_run_workflow(&self, input: &AgentRunWorkflowInput) -> Result<StartWorkflowExecutionResponse>;
}
```

### TemporalWorker (`temporal_worker.rs`)

Worker that polls Temporal for tasks and executes them.

```rust
pub struct TemporalWorker {
    core: Arc<TemporalWorkerCore>,
    db: Database,
}

impl TemporalWorker {
    async fn run(&self) -> Result<()>;  // Polls until shutdown
}
```

### Workflow State Machine (`workflows_temporal.rs`)

The agent run workflow as a deterministic state machine.

```rust
pub enum AgentRunWorkflowState {
    Starting,
    LoadingAgent { activity_seq: u32 },
    LoadingMessages { activity_seq: u32, agent_config: LoadAgentOutput },
    CallingLlm { activity_seq: u32, agent_config: LoadAgentOutput, messages: Vec<MessageData>, iteration: u8 },
    ExecutingTools { activity_seq: u32, agent_config: LoadAgentOutput, messages: Vec<MessageData>, tool_calls: Vec<ToolCallData>, iteration: u8 },
    SavingMessage { ... },
    UpdatingStatus { activity_seq: u32, final_status: String },
    Completed,
    Failed { error: String },
}
```

### Activities (`activities_temporal.rs`)

Idempotent activity functions:

| Activity | Description | Heartbeat |
|----------|-------------|-----------|
| `load_agent` | Load agent configuration from database | No |
| `load_messages` | Load thread messages | No |
| `update_status` | Update run status in database | No |
| `persist_event` | Persist AG-UI event to database | No |
| `call_llm` | Call LLM and stream response | Yes (every 10 chunks) |
| `execute_tools` | Execute tool calls | Yes (per tool) |
| `save_message` | Save message to thread | No |

## Workflow Execution Flow

1. **API receives request** - Creates run in database
2. **TemporalRunner.start_run()** - Queues workflow to Temporal server
3. **Worker polls workflow task** - Creates workflow state machine
4. **Workflow schedules activities** - update_status, load_agent
5. **Worker polls activity task** - Executes load_agent
6. **Activity result returned** - Workflow transitions to LoadingMessages
7. **LLM call with heartbeats** - call_llm activity streams and persists events
8. **Tool iteration** - If tool calls present, execute_tools then call_llm again
9. **Completion** - save_message, update_status to completed
10. **Workflow completes** - Run marked complete in database

## Streaming Preservation

SSE streaming to clients is preserved through database-backed events:

1. Activities persist events to `run_events` table using `PersistEventActivity`
2. `call_llm` activity persists `TEXT_MESSAGE_CONTENT` events during streaming
3. SSE endpoint polls `run_events` table for new events
4. No changes needed to SSE infrastructure

## Configuration

Environment variables:

```bash
AGENT_RUNNER_MODE=temporal          # Enable Temporal mode
TEMPORAL_ADDRESS=localhost:7233     # Temporal server address
TEMPORAL_NAMESPACE=default          # Temporal namespace
TEMPORAL_TASK_QUEUE=everruns-agent-runs  # Task queue name
```

## In-Process Mode (Default)

When `AGENT_RUNNER_MODE=inprocess` (default), execution happens in the API process:

- Workflows are tokio tasks
- Same activity logic reused
- Good for development and single-process deployment

## Error Handling

### Activity Failures
- Temporal automatically retries failed activities
- Non-retryable errors transition workflow to Failed state
- Failed workflows update run status to "failed"

### Workflow Failures
- Workflow failures are recorded in database
- AG-UI `RUN_ERROR` event emitted
- Run status updated to "failed"

## Activity Heartbeats

Long-running activities use heartbeats to report progress:

```rust
// In call_llm activity
if chunk_count % 10 == 0 {
    ctx.heartbeat(&format!("Streaming LLM response: {} tokens", full_response.len()));
}
```

Heartbeat timeout is 30 seconds. If an activity fails to heartbeat, Temporal will schedule it for retry.

## Tool Iteration Limit

Maximum 10 tool iterations per run to prevent infinite loops:

```rust
const MAX_TOOL_ITERATIONS: u8 = 10;
```

## Database Schema Usage

| Table | Usage |
|-------|-------|
| `runs` | Status updates, workflow_id recording |
| `run_events` | AG-UI event persistence for streaming |
| `messages` | Thread message storage |
| `agents` | Agent configuration loading |

## SDK Choice

Using `temporal-sdk-core` v0.1.0-alpha.1 which provides:
- gRPC client for Temporal server
- Workflow and activity task polling
- Proto definitions for Temporal API

The SDK is pre-alpha but provides the necessary primitives. If stability issues arise, direct gRPC usage is possible since we've isolated Temporal behind a clean interface.
