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
| `persist_output` | Tool requests full output be persisted to VFS before truncation when supported | Assume no persistence |

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
3. **Truncate** — priority-aware truncation at the budget determined by the `output` verbosity parameter

### Output Verbosity (EVE-236)

All exec tools accept an `output` parameter controlling how much output is returned to the LLM:

| Mode | Budget | When to use |
|------|--------|-------------|
| `silent` | ~200 B | Minimal truncated output — fire-and-forget commands |
| `concise` | ~2 KiB | Tail ~30 lines — builds, installs, known-good commands (**default**) |
| `normal` | ~8 KiB | General use, debugging |
| `verbose` | ~16 KiB | Test failures, error investigation |
| `full` | unlimited | Raw output, no truncation — when the LLM needs every line |

Default is `concise`. Budgets apply to stdout; stderr is capped at `min(budget, 4096)` to keep error output proportional. Full output is always persisted to `/.outputs/` via `tool_output_persistence` — stdout to `/.outputs/{tool_call_id}.stdout`, stderr to `.stderr` — and readable with `read_file`. See `crates/core/src/tool_output_sanitizer.rs` for budget constants and `output_verbosity_budget()`.

This is the tool's responsibility — each tool calls the helpers before constructing `ToolExecutionResult`. See `crates/core/src/tool_output_sanitizer.rs` for the primitives.

### PostToolExecHook (per-tool hooks)

`PostToolExecHook` is an async hook that runs after each individual tool execution, before ActAtom emits events. Capabilities contribute hooks via `Capability::post_tool_exec_hooks()`.

Two hook slots run in sequence:
1. **Capability hooks** (`post_tool_hooks`) — from active capabilities (e.g. `tool_output_persistence`)
2. **Final hooks** (`final_post_tool_hooks`) — always-on infrastructure (e.g. EVE-225 hard limit)

Current hooks:
- **PersistOutputHook** (`tool_output_persistence` capability, included in Generic harness): When a tool declares `persist_output: true` in hints, writes stdout to `/.outputs/{tool_call_id}.stdout` and stderr to `/.outputs/{tool_call_id}.stderr` in session VFS, injecting `full_output`, `total_lines`, and `output_files` into the result. See `crates/core/src/capabilities/tool_output_persistence.rs`.

Current final hooks (always-on, cannot be removed):
- **OutputHardLimitHook** (EVE-225): Enforces a 64 KiB hard ceiling on serialized tool result text. Head-truncation with UTF-8 safety; appends an LLM-actionable suffix. Logs `tracing::warn!` with tool_name, tool_call_id, result_bytes, limit when truncating. Fires regardless of which capabilities are active. See `crates/core/src/atoms/act_hooks.rs`.

### Loop Detection (EVE-227)

The `loop_detection` capability detects repeated identical tool calls and injects a system warning to break the loop. It uses `MessageFilterProvider::post_load` to scan loaded messages.

**Mechanism:** After messages are loaded, the filter scans recent agent messages in reverse. If `threshold` (default 3) consecutive agent messages carry tool calls with identical signatures (name + arguments, order-independent), a system message is appended warning the model to try a different approach.

**Configuration:** `{"threshold": 5}` to change the default repeat count.

See `crates/core/src/capabilities/loop_detection.rs`.

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
