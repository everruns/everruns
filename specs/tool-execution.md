# Tool Execution Specification

## Abstract

Everruns agents can invoke tools during execution. This specification defines tool types, execution policies, and the tool calling loop behavior.

## Requirements

### Tool Types

#### Built-in Tools
System-provided tools implemented via the `Tool` trait in `everruns-core`.

See `crates/core/src/tools.rs` for the `Tool` trait, `ToolExecutionResult`, and `ToolPolicy` types.

**Display Names:**
Tools should provide a human-readable `display_name` for UI rendering. Propagated through events; UI falls back to technical `name` if absent.

**Error Handling Contract:**
- `Success(Value)` — Result returned to LLM
- `SuccessWithImages { result, images }` — Result with native image content blocks
- `ToolError(String)` — User-visible error as `{"error": "..."}`
- `InternalError` — System error logged, generic message (security)

All error types continue the agent loop — packaged in `result` field, LLM decides how to proceed.

**Image Support in Tool Results:**

Tools can return images alongside JSON results via `ToolExecutionResult::success_with_images()`. Images are represented as `ToolResultImage { base64, media_type }` and flow through the system as native image content:

1. `ToolExecutionResult::SuccessWithImages` → `ToolResult` with `images: Option<Vec<ToolResultImage>>`
2. `ToolResult.images` → `ContentPart::Image` in tool result messages
3. `ContentPart::Image` → `LlmContentPart::Image` in LLM messages
4. Provider-specific formatting: Anthropic uses array content blocks in `tool_result`, OpenAI uses `image_url` content parts

Supported image formats: PNG, JPEG, GIF, WebP (matching LLM vision API support).

Note: OpenAI Responses API (`function_call_output`) does not support images in tool results - images are dropped with a warning in that path.

**Provided Tools:**
- `GetCurrentTime` - Returns current timestamp in various formats (iso8601, unix, human)
- `EchoTool` - Echoes input (useful for testing)
- `FailingTool` - Always fails (for error handling tests)

**Capability-Provided Tools:**

Tools can also be provided by Capabilities (see [capabilities.md](capabilities.md)). When an agent has capabilities enabled, their tools are merged into the agent's tool set:

- `CurrentTime` capability provides `get_current_time` tool
- `WebFetch` capability provides `web_fetch` tool for fetching URL content and converting HTML to markdown/text
- `Research` capability will provide scratchpad and search tools
- `Sandbox` capability will provide `execute_code` tool
- `FileSystem` capability will provide read/write/search files tools

**ToolRegistry:** Manages multiple tools and implements `ToolExecutor` trait. See `crates/core/src/tools.rs` for `ToolRegistry` and its builder pattern.

### Tool Policies

- `auto`: Execute immediately without approval
- `requires_approval`: Pause and wait for user approval (HITL - future)

### Execution Flow

1. LLM returns tool calls in response
2. For each tool call:
   - Emit `ToolCallStart` event
   - Execute tool via `ToolRegistry`
   - Emit `ToolCallResult` event
3. Add tool results to message history
4. Call LLM again with results
5. Repeat until LLM returns final response (max 10 iterations)

### Step-Based Execution (Durable Mode)

In durable mode, each LLM call and each tool call is a **separate durable activity (task)**:

```
┌─────────────┐
│ SetupStep   │ → Load agent config + messages
└─────────────┘
       ↓
┌─────────────────┐
│ ExecuteLlmStep  │ → Call LLM (iteration 1)
└─────────────────┘
       ↓ (if tool calls)
┌───────────────────────┐   ┌───────────────────────┐
│ ExecuteSingleTool #1  │ → │ ExecuteSingleTool #2  │ → ...
└───────────────────────┘   └───────────────────────┘
       ↓ (loop back)
┌─────────────────┐
│ ExecuteLlmStep  │ → Call LLM (iteration 2)
└─────────────────┘
       ↓ (no tools)
┌──────────────┐
│ FinalizeStep │ → Save final message, update status
└──────────────┘
```

Benefits:
- **Individual retries**: Failed tool can retry without re-running LLM
- **Maximum observability**: Each step visible in workflow event log
- **Better debugging**: Isolate failures to specific steps

### Security

1. **Tool Validation**: Only registered tools can be executed
2. **Policy Enforcement**: `requires_approval` tools pause for user confirmation (future)
3. **Rate Limiting**: Per-agent rate limits (future)
