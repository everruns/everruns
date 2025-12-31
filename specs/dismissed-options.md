# Dismissed Options

This document records technical options that were considered but dismissed for specific reasons. These decisions may be revisited in the future as circumstances change.

## AG-UI Protocol

**Status**: Dismissed (may revisit)

**What it was**: AG-UI is a protocol for streaming agent UI events, designed for compatibility with CopilotKit and other agent UI frameworks. See https://docs.ag-ui.com for the specification.

**Why considered**: AG-UI provided a standardized event format for streaming agent execution events (RunStarted, TextMessageContent, ToolCallStart, etc.) to UI clients via SSE. This would enable compatibility with the CopilotKit ecosystem and other AG-UI-compatible clients.

**Why dismissed**: The current implementation priorities shifted away from CopilotKit compatibility. The system uses Temporal workflows for orchestration, which provides sufficient visibility into workflow execution state without a separate event streaming layer.

**What we use instead**: Temporal workflow state for tracking execution progress. Session status transitions (pending → running → pending) reflect workflow state changes. Real-time streaming can be revisited when there's a concrete need.

**May revisit when**:
- There is renewed interest in CopilotKit integration
- Real-time SSE streaming becomes a requirement
- The AG-UI protocol matures and provides clear benefits
