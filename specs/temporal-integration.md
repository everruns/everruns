# Temporal Integration Specification

## Abstract

This specification documents the Temporal workflow integration for agent execution in Everruns. All agent workflows run via Temporal for durability and reliability.

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

### TemporalClient (`client.rs`)

Client wrapper for starting workflows from the API process.

```rust
pub struct TemporalClient {
    gateway: Arc<ServerGateway>,
    config: RunnerConfig,
}

impl TemporalClient {
    async fn start_turn_workflow(&self, input: &TurnWorkflowInput) -> Result<StartWorkflowExecutionResponse>;
}
```

### TemporalWorker (`worker.rs`)

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

### TurnWorkflow State Machine (`turn_workflow.rs`)

Turn-based workflow implementation with trait-based abstraction:

```rust
pub trait Workflow: Send + Sync + Debug {
    fn workflow_type(&self) -> &'static str;
    fn on_start(&mut self) -> Vec<WorkflowAction>;
    fn on_activity_completed(&mut self, activity_id: &str, result: Value) -> Vec<WorkflowAction>;
    fn on_activity_failed(&mut self, activity_id: &str, error: &str) -> Vec<WorkflowAction>;
    fn is_completed(&self) -> bool;
}
```

TurnWorkflow states:
- `Init` - Initial state, schedules Input activity
- `ProcessingInput` - Waiting for InputAtom to retrieve user message
- `Reasoning` - Waiting for ReasonAtom to call LLM
- `Acting` - Waiting for ActAtom to execute tools
- `Completed` - Workflow finished successfully
- `Failed` - Workflow encountered an error

### Activities (`activities.rs`)

Turn-based activity functions using Atoms:

| Activity | Atom | Description |
|----------|------|-------------|
| `input` | `InputAtom` | Retrieve user message from store |
| `reason` | `ReasonAtom` | Call LLM with context, store response |
| `act` | `ActAtom` | Execute tools in parallel, store results |

### Atoms (`everruns-core/atoms/`)

Stateless atomic operations:

| Atom | Input | Output | Description |
|------|-------|--------|-------------|
| `InputAtom` | `AtomContext` | `InputAtomResult` | Retrieves user message by ID |
| `ReasonAtom` | `AtomContext`, `agent_id` | `ReasonResult` | Prepares context, calls LLM, stores response |
| `ActAtom` | `AtomContext`, `tool_calls`, `tool_definitions` | `ActResult` | Executes tools in parallel |

## AtomContext

Each atom execution receives an `AtomContext` for tracking:

```rust
pub struct AtomContext {
    pub session_id: Uuid,        // The conversation session
    pub turn_id: Uuid,           // Unique identifier for this turn
    pub input_message_id: Uuid,  // User message that triggered this turn
    pub exec_id: Uuid,           // Unique identifier for this atom execution
}
```

## Workflow Execution Flow

1. **API receives message** - Creates user message event in database
2. **MessageService.start_workflow()** - Passes `input_message_id` to runner
3. **TemporalRunner.start_run()** - Queues TurnWorkflow to Temporal server
4. **Worker polls workflow task** - Creates TurnWorkflow state machine
5. **Input phase** - InputAtom retrieves user message from store
6. **Reason phase** - ReasonAtom calls LLM, stores assistant response
7. **Act phase** - If tool calls present, ActAtom executes tools in parallel
8. **Loop** - Repeat Reason → Act until no more tool calls
9. **Completion** - Workflow completes, session status updated

```
TurnWorkflow Execution:
┌──────────┐     ┌──────────────────┐     ┌───────────┐
│   Init   │────►│ ProcessingInput  │────►│ Reasoning │
└──────────┘     └──────────────────┘     └─────┬─────┘
                                                │
                     ┌──────────────────────────┼──────────────────┐
                     │                          │                  │
                     ▼                          ▼                  ▼
              ┌───────────┐              ┌───────────┐      ┌───────────┐
              │ Completed │◄─────────────│  Acting   │◄─────│  Failed   │
              │(no tools) │              │           │      │           │
              └───────────┘              └─────┬─────┘      └───────────┘
                                               │
                                               │ (has more tools)
                                               ▼
                                         ┌───────────┐
                                         │ Reasoning │
                                         │  (loop)   │
                                         └───────────┘
```

## Streaming Preservation

SSE streaming to clients is preserved through database-backed events:

1. Activities persist events to `events` table
2. ReasonAtom persists assistant message events during LLM response
3. ActAtom persists tool result events
4. SSE endpoint polls events table for new events
5. No changes needed to SSE infrastructure

## Configuration

Environment variables:

```bash
TEMPORAL_ADDRESS=localhost:7233          # Temporal server address
TEMPORAL_NAMESPACE=default               # Temporal namespace
TEMPORAL_TASK_QUEUE=everruns-agent-runs  # Task queue name
```

## Error Handling

### Activity Failures
- Temporal automatically retries failed activities
- Non-retryable errors transition workflow to Failed state
- Failed workflows update session status to "failed"

### Atom Error Handling
- Errors in atoms are returned as normal results (not exceptions)
- ReasonResult and ActResult include `error` and `success` fields
- Workflow decides how to handle errors based on result

### Workflow Failures
- Workflow failures are recorded in database
- `SESSION_ERROR` event emitted
- Session status updated to "failed"

## Tool Iteration Limit

Maximum iterations configurable via ReasonResult.max_iterations (default: 100):

```rust
pub struct ReasonResult {
    // ...
    pub max_iterations: u32,  // Default 100
}
```

TurnWorkflow tracks iteration count and fails if exceeded.

## Database Schema Usage

| Table | Usage |
|-------|-------|
| `sessions` | Status updates, workflow_id recording |
| `events` | Event persistence for streaming (messages stored as events) |
| `agents` | Agent configuration loading |

## SDK Choice

Using `temporal-sdk-core` v0.1.0-alpha.1 which provides:
- gRPC client for Temporal server
- Workflow and activity task polling
- Proto definitions for Temporal API

The SDK is pre-alpha but provides the necessary primitives. If stability issues arise, direct gRPC usage is possible since we've isolated Temporal behind a clean interface.
