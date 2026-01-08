# Dismissed Options

This document records technical options that were considered but dismissed for specific reasons. These decisions may be revisited in the future as circumstances change.

## AG-UI Protocol

**Status**: Dismissed (may revisit)

**What it was**: AG-UI is a protocol for streaming agent UI events, designed for compatibility with CopilotKit and other agent UI frameworks. See https://docs.ag-ui.com for the specification.

**Why considered**: AG-UI provided a standardized event format for streaming agent execution events (RunStarted, TextMessageContent, ToolCallStart, etc.) to UI clients via SSE. This would enable compatibility with the CopilotKit ecosystem and other AG-UI-compatible clients.

**Why dismissed**: The current implementation priorities shifted away from CopilotKit compatibility. The system uses a custom PostgreSQL-backed durable execution engine for orchestration, which provides sufficient visibility into workflow execution state without a separate event streaming layer.

**What we use instead**: PostgreSQL-backed durable execution state for tracking execution progress. Session status transitions (pending → running → pending) reflect workflow state changes. Real-time streaming can be revisited when there's a concrete need.

**May revisit when**:
- There is renewed interest in CopilotKit integration
- Real-time SSE streaming becomes a requirement
- The AG-UI protocol matures and provides clear benefits

## Temporal Workflow Engine

**Status**: Dismissed (implemented then removed)

**What it was**: Temporal is a workflow orchestration platform that provides durable execution guarantees. It was used as the execution backend for agent workflows, with the Temporal SDK integrated into the worker crate.

**Why considered**: Temporal provided battle-tested durable execution at scale with features like:
- Workflow and activity orchestration
- Automatic retry policies
- Workflow history persistence
- Task queues for worker distribution
- Built-in observability and UI

**Why dismissed**: After implementing Temporal integration, we found it added significant operational complexity for our use case:
- Required running a separate Temporal server (with its own database schema)
- SDK was in alpha state (temporal-sdk-core 0.1.0-alpha.1) with stability concerns
- Debugging workflow issues required understanding Temporal internals
- The protobuf compilation dependencies added build complexity
- For our agent workloads, simpler PostgreSQL-backed durability was sufficient

**What we use instead**: A custom PostgreSQL-backed durable execution engine (`everruns-durable` crate):
- Task queue table with optimistic locking for work distribution
- Direct database state for durability guarantees
- Workers communicate with control-plane via gRPC
- Simpler operational model (just PostgreSQL)
- Faster iteration and easier debugging

**May revisit when**:
- Scale requirements exceed what PostgreSQL-based queuing can handle
- Need for complex workflow patterns (compensation, saga, long-running timers)
- Temporal SDK reaches stable release status
- Multi-region deployment requires sophisticated task routing
