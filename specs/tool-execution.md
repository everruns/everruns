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

**Provided Tools:** See `crates/core/src/tools.rs` for the built-in tool implementations.

**Capability-Provided Tools:** Capabilities contribute tools to agents. See [capabilities.md](capabilities.md) and `crates/core/src/capabilities/` for per-capability tool definitions.

**ToolRegistry:** Manages multiple tools and implements `ToolExecutor` trait. See `crates/core/src/tools.rs` for `ToolRegistry` and its builder pattern.

### Tool Hints

`ToolHints` provides semantic metadata about a tool's behavioral properties. See `crates/core/src/tool_types.rs` for the struct definition. All fields are `Option<bool>` — `None` means unspecified.

Follows the [MCP tool annotations](https://spec.modelcontextprotocol.io) convention plus everruns-specific hints:

| Hint | Meaning | Conservative default (when `None`) |
|------|---------|-------------------------------------|
| `readonly` | Tool does not modify any state | Assume not readonly |
| `destructive` | Tool may irreversibly destroy data | Assume not destructive |
| `idempotent` | Same args → same effect; safe to retry | Assume not idempotent |
| `open_world` | Interacts with external entities (network, APIs) | Assume closed-world |
| `requires_secrets` | Needs API keys or credentials | Assume no secrets needed |
| `long_running` | May take significant time (> ~5s typical) | Assume fast |
| `persist_output` | Full output persisted to VFS before truncation | Assume no persistence |

**Design rules:**
- Hints are informational — they do not enforce policy. Use `ToolPolicy` for execution gating.
- Tools should set hints via the `Tool::hints()` trait method or directly on `BuiltinTool.hints`.
- MCP server tools inherit hints from MCP `annotations` when available.
- External toolkit libraries expose hints via the `Tool::hints()` method in the toolkit library contract.
- `destructive` is a subset of non-readonly — a tool can write without being destructive.

### Exec Tool Output Sanitization

Exec tools (bash, daytona_exec, e2b_exec, deno_exec, sprites_exec, docker_exec) sanitize their output before returning results. Each tool calls `sanitize_exec_output()` from `crates/core/src/tool_output_sanitizer.rs`.

The pipeline:
1. **Strip ANSI** — remove SGR, CSI, OSC escape sequences
2. **Collapse CR lines** — `\r`-overwritten lines (progress bars) reduced to final content
3. **Middle-truncate** — 20% head / 80% tail split at 16 KiB budget (errors cluster at the end)

This is the tool's responsibility — each tool calls the helpers before constructing `ToolExecutionResult`. See `crates/core/src/tool_output_sanitizer.rs` for the primitives.

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
