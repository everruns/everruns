# Dismissed Options

This document records technical options that were considered but dismissed for specific reasons. These decisions may be revisited in the future as circumstances change.

## AG-UI Protocol

**Status**: Dismissed (may revisit)

**What it was**: AG-UI is a protocol for streaming agent UI events, designed for compatibility with CopilotKit and other agent UI frameworks. See https://docs.ag-ui.com for the specification.

**Why considered**: AG-UI provided a standardized event format for streaming agent execution events (RunStarted, TextMessageContent, ToolCallStart, etc.) to UI clients via SSE. This would enable compatibility with the CopilotKit ecosystem and other AG-UI-compatible clients.

**Why dismissed**: The current implementation priorities shifted away from CopilotKit compatibility. The internal event system (LoopEvent) provides sufficient streaming capabilities for our needs without the additional abstraction layer.

**What we use instead**: Native LoopEvent types for internal event streaming:
- `LoopStarted`, `LoopCompleted`, `LoopError` - Lifecycle events
- `TextDelta` - Streaming text content
- `ToolExecutionStarted`, `ToolExecutionCompleted` - Tool execution events
- `IterationStarted`, `IterationCompleted` - Loop iteration tracking

**May revisit when**:
- There is renewed interest in CopilotKit integration
- Other AG-UI-compatible clients need to be supported
- The AG-UI protocol matures and provides clear benefits over our native events
