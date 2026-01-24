# AG-UI Protocol Test Cases

## TC-AGUI-001: Basic Text Message Flow

**Objective**: Verify AG-UI SSE endpoint streams text message events correctly.

**Prerequisites**:
- Everruns running: `just start-dev --no-watch`
- LLM configured (or using LLMSim)

**Steps**:
1. Create agent and session
2. Connect to `/ag-ui/sse` endpoint
3. Send a simple text message
4. Observe SSE events

**Expected Events** (in order):
1. `connected` - Initial connection event
2. `RUN_STARTED` - Turn started with `threadId` and `runId`
3. `TEXT_MESSAGE_START` - Message with `messageId` and `role: "assistant"`
4. `TEXT_MESSAGE_CONTENT` - One or more events with `delta` field
5. `TEXT_MESSAGE_END` - Message completed with same `messageId`
6. `RUN_FINISHED` - Turn completed with same `threadId` and `runId`

**Validation**:
- [ ] All events have `type` field in SCREAMING_SNAKE_CASE
- [ ] All events have `timestamp` field (optional)
- [ ] `threadId` matches session ID
- [ ] `runId` matches turn ID
- [ ] `messageId` is consistent across message events

---

## TC-AGUI-002: Tool Call Flow

**Objective**: Verify tool calls are properly translated to AG-UI format.

**Prerequisites**:
- Everruns running with tool-enabled agent (e.g., `current_time` capability)

**Steps**:
1. Create agent with `current_time` capability
2. Create session
3. Connect to `/ag-ui/sse` endpoint
4. Send message: "What time is it?"
5. Observe SSE events

**Expected Events**:
1. `RUN_STARTED`
2. `TOOL_CALL_START` with `toolCallId` and `toolCallName: "current_time"`
3. `TOOL_CALL_ARGS` with tool arguments JSON
4. `TOOL_CALL_END` with same `toolCallId`
5. `TOOL_CALL_RESULT` with `result` field
6. `TEXT_MESSAGE_START`
7. `TEXT_MESSAGE_CONTENT` (one or more)
8. `TEXT_MESSAGE_END`
9. `RUN_FINISHED`

**Validation**:
- [ ] `toolCallId` is consistent across tool events
- [ ] `toolCallName` matches the invoked tool
- [ ] `result` contains tool output

---

## TC-AGUI-003: Error Handling

**Objective**: Verify errors are translated to `RUN_ERROR` events.

**Prerequisites**:
- Everruns running
- Agent configured with missing/invalid LLM provider

**Steps**:
1. Create agent without valid LLM configuration
2. Create session
3. Connect to `/ag-ui/sse` endpoint
4. Send a message
5. Observe SSE events

**Expected Events**:
1. `RUN_STARTED`
2. `RUN_ERROR` with `message` field and optional `code` field

**Validation**:
- [ ] `RUN_ERROR` contains meaningful error message
- [ ] `code` field present (e.g., "llm_error")

---

## TC-AGUI-004: Extended Thinking Events

**Objective**: Verify extended thinking is translated to AG-UI thinking events.

**Prerequisites**:
- Everruns running
- Anthropic Claude model with `reasoning_effort` configured

**Steps**:
1. Create agent with Claude model and `reasoning_effort: "medium"`
2. Create session
3. Connect to `/ag-ui/sse` endpoint
4. Send a complex reasoning question
5. Observe SSE events

**Expected Events**:
1. `RUN_STARTED`
2. `THINKING_TEXT_MESSAGE_START` with `messageId`
3. `THINKING_TEXT_MESSAGE_CONTENT` (one or more) with `delta`
4. `THINKING_TEXT_MESSAGE_END` with same `messageId`
5. `TEXT_MESSAGE_START`
6. `TEXT_MESSAGE_CONTENT` (one or more)
7. `TEXT_MESSAGE_END`
8. `RUN_FINISHED`

**Validation**:
- [ ] Thinking events have distinct `messageId` from response
- [ ] Thinking content represents chain-of-thought reasoning

---

## TC-AGUI-005: Cancellation

**Objective**: Verify turn cancellation produces `RUN_ERROR` with cancellation code.

**Prerequisites**:
- Everruns running
- Agent with slow response (e.g., tool that takes time)

**Steps**:
1. Create agent and session
2. Connect to `/ag-ui/sse` endpoint
3. Send a message that triggers slow operation
4. Cancel the turn via API
5. Observe SSE events

**Expected Events**:
1. `RUN_STARTED`
2. (Possibly some content events)
3. `RUN_ERROR` with `code: "cancelled"`

**Validation**:
- [ ] `RUN_ERROR` has `code: "cancelled"`
- [ ] `message` indicates cancellation reason

---

## TC-AGUI-006: SSE Reconnection

**Objective**: Verify `since_id` parameter allows reconnection without duplicate events.

**Prerequisites**:
- Everruns running

**Steps**:
1. Create agent and session
2. Connect to `/ag-ui/sse`, record last event ID
3. Disconnect
4. Send another message while disconnected
5. Reconnect with `since_id={last_event_id}`
6. Observe events

**Expected Behavior**:
- Only events after `since_id` are delivered
- No duplicate events

---

## TC-AGUI-007: Internal Events Not Leaked

**Objective**: Verify internal events are not exposed via AG-UI endpoint.

**Prerequisites**:
- Everruns running

**Steps**:
1. Create agent and session
2. Connect to both `/sse` and `/ag-ui/sse` endpoints
3. Send a message
4. Compare events from both endpoints

**Expected Behavior**:
- Native `/sse` shows: `reason.started`, `reason.completed`, `act.started`, `act.completed`, `session.activated`, `session.idled`, etc.
- AG-UI `/ag-ui/sse` does NOT show these internal events
- AG-UI only shows: `RUN_*`, `TEXT_MESSAGE_*`, `TOOL_CALL_*`, `THINKING_*`

---

## TC-AGUI-008: CopilotKit Demo

**Objective**: Verify the CopilotKit demo application works correctly.

**Prerequisites**:
- Everruns running: `just start-dev --no-watch`
- Node.js 18+

**Steps**:
1. Navigate to `examples/copilotkit-demo/`
2. Run `npm install`
3. Run `npm run dev`
4. Open http://localhost:5173
5. Wait for "Initializing..." to complete
6. Type a message and press Send
7. Observe chat and events panel

**Expected Behavior**:
- Agent and session IDs shown in events panel
- Messages appear in chat
- AG-UI events visible in events panel
- Streaming text updates in real-time

**Validation**:
- [ ] Chat messages render correctly
- [ ] Events panel shows AG-UI event types
- [ ] No JavaScript console errors
