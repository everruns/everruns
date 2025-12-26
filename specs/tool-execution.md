# Tool Execution Specification

## Abstract

Everruns agents can invoke tools during execution. This specification defines tool types, execution policies, and the tool calling loop behavior.

## Requirements

### Tool Types

#### Built-in Tools
System-provided tools implemented via the `Tool` trait in `everruns-core`.

**Tool Trait Interface:**
```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, arguments: Value) -> ToolExecutionResult;
    fn policy(&self) -> ToolPolicy { ToolPolicy::Auto }
}
```

**Error Handling Contract:**
- `ToolExecutionResult::Success(Value)` - Successful result returned to LLM
- `ToolExecutionResult::ToolError(String)` - User-visible error shown to LLM (e.g., "City not found")
- `ToolExecutionResult::InternalError` - System error logged but hidden from LLM (security)

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

**ToolRegistry:**
Manages multiple tools and implements `ToolExecutor` trait for integration with `AgentLoop`:
```rust
// Create with default built-in tools (includes get_current_time, echo, math tools, etc.)
let registry = ToolRegistry::with_defaults();

// Or build a custom registry
let registry = ToolRegistry::builder()
    .tool(GetCurrentTime)
    .tool(MyCustomTool)
    .build();

let agent_loop = AgentLoop::new(config, emitter, store, llm, registry);
```

### Tool Definition Schema

```json
{
  "type": "builtin",
  "name": "tool_name",
  "description": "What the tool does",
  "parameters": {
    "type": "object",
    "properties": {
      "param1": {
        "type": "string",
        "description": "Parameter description"
      }
    },
    "required": ["param1"]
  },
  "kind": "current_time",
  "policy": "auto"
}
```

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

### Step-Based Execution (Temporal Mode)

In Temporal mode, each LLM call and each tool call is a **separate Temporal activity (node)**:

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
- **Maximum observability**: Each step visible in Temporal UI
- **Better debugging**: Isolate failures to specific steps

### Security

1. **Tool Validation**: Only registered tools can be executed
2. **Policy Enforcement**: `requires_approval` tools pause for user confirmation (future)
3. **Rate Limiting**: Per-agent rate limits (future)
