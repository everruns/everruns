# Everruns + CopilotKit Demo

This demo application shows how to use Everruns with the AG-UI protocol and CopilotKit.

## Prerequisites

- Node.js 18+
- Everruns API running at `http://localhost:9000`

## Quick Start

1. **Start Everruns API**

   ```bash
   # From the repository root
   just start-dev --no-watch
   ```

2. **Install dependencies**

   ```bash
   cd examples/copilotkit-demo
   npm install
   ```

3. **Start the demo**

   ```bash
   npm run dev
   ```

4. **Open in browser**

   Visit http://localhost:5173

## Features

- Real-time streaming via AG-UI protocol SSE endpoint
- Event visualization panel showing raw AG-UI events
- Chat interface with message history
- Automatic agent and session creation

## AG-UI Events

The demo subscribes to the following AG-UI events:

| Event | Description |
|-------|-------------|
| `RUN_STARTED` | Turn/run has started |
| `RUN_FINISHED` | Turn/run has completed |
| `RUN_ERROR` | Turn/run failed with error |
| `TEXT_MESSAGE_START` | Assistant message started |
| `TEXT_MESSAGE_CONTENT` | Streaming text content |
| `TEXT_MESSAGE_END` | Assistant message completed |
| `TOOL_CALL_START` | Tool call initiated |
| `TOOL_CALL_RESULT` | Tool call result received |
| `THINKING_TEXT_MESSAGE_*` | Extended thinking events |

## Architecture

```
┌─────────────────┐    SSE (AG-UI)    ┌─────────────────┐
│  React App      │◄─────────────────│  Everruns API   │
│  (CopilotKit)   │───────────────────►│  /ag-ui/sse     │
└─────────────────┘   POST /messages  └─────────────────┘
```

## Development

```bash
# Development server with hot reload
npm run dev

# Type check
npm run build

# Preview production build
npm run preview
```

## Integration with CopilotKit

This demo uses the AG-UI protocol which is compatible with CopilotKit. To use CopilotKit's
built-in components:

```tsx
import { CopilotKit } from '@copilotkit/react-core'
import { CopilotSidebar } from '@copilotkit/react-ui'

function App() {
  return (
    <CopilotKit runtimeUrl="/api/copilot">
      <CopilotSidebar />
      {/* Your app content */}
    </CopilotKit>
  )
}
```

For full CopilotKit integration, you would need to implement a runtime adapter that
translates between CopilotKit's expected format and Everruns' AG-UI endpoint.
