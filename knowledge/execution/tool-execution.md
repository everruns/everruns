---
type: Specification
title: "Tool Execution Specification"
description: "Tool types and execution flow."
tags:
  - everruns
  - execution
---
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

`ToolHints` provides semantic metadata about a tool's behavioral properties. See `crates/provider/src/tool_types.rs` for the struct definition. The behavioral hints are all `Option<bool>` — `None` means unspecified.

Alongside them, `metadata: Option<Value>` is an opaque hatch for annotations **core does not interpret**: risk tiers for an approval UI, presentation hints, an embedder's routing keys. The typed hints are the vocabulary core reasons about; the hatch is how a host carries its own without a core patch per field. No driver sends it to a provider, and it must never carry credentials or other sensitive payload — it is persisted and surfaced to clients like the rest of the definition. `Capability::metadata()` is the same hatch one level up.

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
| `supports_background` | Tool can be executed through `spawn_background` and stream status/output/progress updates | Assume foreground-only |
| `concurrency_class` | Scheduling conflict key; calls sharing a non-empty class serialize within an act batch (see [Tool Scheduling](#tool-scheduling)) | No class → no conflicts, always parallelizable |
| `cpu_bound` | Tool does significant non-yielding in-process work; the scheduler offloads it to its own task | Assume I/O-bound (cooperative) |
| `narration_noun` | Entity noun for operation-based narration (e.g. `"agent"`, `"harness"`) | Generic "Ran {display_name}" fallback |

### Narration Formatting

Every tool call displayed in the UI gets a human-readable narration line (e.g. "Created agent: Neon Cartographer"). Narration is **owned by the tool**, via `Tool::narrate`, and surfaced by its capability — there is no central name-keyed narrator. everruns authors narration in the backend so downstream clients can render with `data.narration.unwrap_or(display_name)` and need no per-tool narration code. See [`knowledge/execution/tool-narration.md`](tool-narration.md) for the contract, wiring, reusable phrasing helpers, and the truncation/redaction rules.

**Tool-owned narration:** A tool implements `Tool::narrate` (calling the reusable phrasing helpers in `crate::tool_narration`); `Capability::narrate` defaults to dispatching to the matching tool, so a capability narrates its tools for free. A capability overrides `narrate()` only when narration is config-driven, spans tools, or the tools are dynamic (e.g. proxied MCP tools). The framework routes every applied capability's `narrate()` through the act atom. Tools that contribute no narration fall through to the generic `narration_noun`/display-name path below.

**Operation-based narration via `narration_noun`:** Multi-operation tools (CRUD tools with an `operation`/`action` argument) should set `narration_noun` in their `ToolHints`. The generic fallback then:

1. Reads the `operation` (or `action`) argument value
2. Maps it to verb forms: create→Creating/Created, update→Updating/Updated, delete→Deleting/Deleted, copy→Copying/Copied, etc.
3. Reads a display name from `name`, `title`, or `new_name` arguments
4. Produces: `"{Verb} {noun}: {name}"` or `"{Verb} {noun}"` if no name

Example: `manage_agents` with `narration_noun: "agent"` and args `{operation: "create", name: "Neon Cartographer"}` → "Created agent: Neon Cartographer".

If `narration_noun` is set but no `operation` argument exists, falls back to generic narration. See `operation_narration()` and `operation_verbs()` in `tool_narration.rs`.

**Requirement:** All new tools with an `operation`/`action` parameter **must** set `narration_noun` in their hints. Tools without operation semantics get reasonable default narration from `display_name` and need no special configuration.

**Model-authored narration via `human_intent`:** The `human_intent` capability uses capability hooks to add an optional `human_intent` string argument to every active tool schema sent to the LLM. The model may fill this with concise user-facing narration of the intended action, such as `"Listing all harnesses"`. A tool-call hook reads this value for backend-authored tool event narration, then strips it from the tool call used for actual execution so built-in and MCP tools do not receive the UI-only argument.

Capability hooks involved:
- `ToolDefinitionHook`: transforms the final merged tool definition list before provider serialization.
- `ToolCallHook`: can read a model-produced tool call for narration and transform the execution copy of that call before invoking the tool.

**Design rules:**
- Hints are informational — they do not enforce policy. Use `ToolPolicy` for execution gating.
- Tools should set hints via the `Tool::hints()` trait method or directly on `BuiltinTool.hints`.
- MCP server tools inherit hints from MCP `annotations` when available.
- External toolkit libraries expose hints via the `Tool::hints()` method in the toolkit library contract.
- `destructive` is a subset of non-readonly — a tool can write without being destructive.

### Reading-tool output contract

Every reading tool attaches a shared `truncation` envelope to its JSON response so LLM callers can detect partial output, understand why it was cut, and resume or fall back without regex-matching human markers. The envelope is additive — existing flat fields like `truncated`, `total_lines`, and `row_count` stay in place for back-compat. File-reading tools, including session-sandbox-backed reads such as `sandbox_read_file`, accept `offset` and `limit` and return only that line window for text files. Non-image binary file reads return metadata by default instead of raw base64 or lossy UTF-8.

See [`crates/core/src/truncation_info.rs`](../../crates/core/src/truncation_info.rs) for the source of truth: `TruncationInfo`, `TruncationReason`, and the `assert_conforms` conformance helper.

**Scope — the reading-tool class:**

| Tool | Reason codes | Resume supported? |
|------|--------------|-------------------|
| `read_file` (session VFS + sandbox) | `line_cap` (with resume), `size_cap` (without resume) | Line cap only — `next_offset` = line number |
| `list_directory` (session VFS) | `item_cap` | Yes — `next_offset` = item offset |
| `grep_files` (session VFS) | `line_cap`, `size_cap` | Match-cap cuts resume at the next match offset; an oversized individual context block may require narrower context |
| `sql_query` | `row_cap` | No — narrow `WHERE`/`LIMIT` |
| `browserless_content` / interaction DOM content | `size_cap` | No — narrow via `browserless_scrape` selectors or shrink the source page |

`next_offset` units are tool-specific — `read_file` uses a line number, `list_directory` uses an item offset, and `grep_files` uses a match offset. Each tool documents its own unit via `resume_hint`.

`grep_files` accepts `before_context` and `after_context` from 0 through 20.
The default zero values retain flat matches. Non-zero values return numbered,
merged context blocks with `is_match` markers. Match pagination is applied
before context expansion, and the primary returned text remains under a 64 KiB
budget.

Platform-management `session_read_messages` is not a filesystem reader, but it follows the same token-economy principle: returned message count and per-message content are bounded by defaults, with explicit caps for larger reads.

Exec tools (`bash`, `*_exec`) keep their existing `truncated`/`total_lines`/`output_files` fields and the `exec_budget` reason is reserved for future migration; they are outside this envelope today because their truncation is priority-aware and persisted-output-backed.

**Envelope shape:**

```json
{
  "truncation": {
    "truncated": true,
    "bytes_returned": 49512,
    "bytes_total": 184221,
    "next_offset": 2000,
    "resume_hint": "call read_file with offset=2000 to resume from line 2001",
    "reason": "line_cap"
  }
}
```

| Field | Presence | Description |
|------|----------|-------------|
| `truncated` | required | `true` if the source exceeded a cap |
| `bytes_returned` | required | Bytes of the response's primary content (the field a caller consumes: `content`, `rows`, `entries`, `matches`) — not the serialized wrapping object |
| `bytes_total` | optional | Total bytes of untruncated source when known |
| `next_offset` | optional | Offset to pass back to resume in-place |
| `resume_hint` | paired with `next_offset` | Human-readable resume instruction |
| `reason` | required | Stable enum: `size_cap`, `line_cap`, `row_cap`, `exec_budget`, `item_cap` |

**Conformance:** tests use `everruns_core::truncation_info::assert_conforms(tool_name, &response)` to validate the envelope. Every reading tool has per-tool unit tests under its own crate's `tests` module.

### Exec Tool Output Sanitization

Exec tools (bash, daytona_exec, e2b_exec, deno_exec, sprites_exec, docker_exec) sanitize their output before returning results. Each tool calls `sanitize_exec_output()` from `crates/core/src/tool_output_sanitizer.rs`.

The pipeline:
1. **Strip ANSI** — remove SGR, CSI, OSC escape sequences
2. **Collapse CR lines** — `\r`-overwritten lines (progress bars) reduced to final content
3. **Truncate** — apply the budget determined by the `output` verbosity parameter. Failed commands prioritize diagnostic regions; successful commands preserve a predictable head/tail window so source or search output containing words such as `error` does not displace leading evidence.

### Output Verbosity (EVE-236, EVE-489)

All exec tools accept an `output` parameter controlling how much output is returned to the LLM:

| Mode | Budget | When to use |
|------|--------|-------------|
| `auto` | compact summary on success (~512 B total inline including the `[full output saved to ...]` pointer) / ~8 KiB on failure | Persistence-first — compact summary when the run succeeds and output is persisted, diagnostic window when it fails (**default**) |
| `silent` | ~200 B | Minimal truncated output — fire-and-forget commands |
| `concise` | ~2 KiB | Tail ~30 lines — builds, installs, known-good commands |
| `normal` | ~8 KiB | General use, debugging |
| `verbose` | ~16 KiB | Test failures, error investigation |
| `full` | unlimited | Raw output, no truncation — when the LLM needs every line |

Default is `auto`. In `auto` mode, the resolved budget depends on the process exit code: successful runs (`exit_code == 0`) collapse to `AUTO_SUCCESS_BUDGET` so the model relies on the persisted log; non-zero exits resolve to `normal` (~8 KiB) so failures stay debuggable in-loop. `AUTO_SUCCESS_BUDGET` is intentionally sized so the inline `stdout` field — including the `[full output saved to ... — use read_file ...]` pointer that `PersistOutputHook` appends — stays around 512 bytes total. The pointer uses the session filesystem's display identity. `raw_output` always carries the full cleaned output for persistence hooks, regardless of mode. The persistence hook reads the original tool-call argument and does not re-resolve explicit `silent`/`concise`/`normal`/`verbose`/`full` modes to `auto`; those modes retain their fixed inline window and ignore exit code.

Budgets apply to stdout; stderr is capped at `min(budget, 4096)` to keep error output proportional. Tools that set the `persist_output` hint persist non-empty full output to `/outputs/` via `tool_output_persistence` — stdout to `/outputs/{tool_call_id}.stdout`, stderr to `/outputs/{tool_call_id}.stderr` — and the files are readable with `read_file`. The persisted files are the source of truth for full logs; the inline payload is sized for next-step reasoning. See `crates/core/src/tool_output_sanitizer.rs` for budget constants, `output_verbosity_budget()`, and `resolve_auto_mode()`.

The shared prompt hints (`EXEC_OUTPUT_HINT`, `READ_ECONOMY_HINT` in `tool_output_sanitizer.rs`) also carry a single-read/contextual-search policy for persisted output (EVE-778): pre-filter in the originating command when the filter is known, read a small persisted log (≤200 lines or ≤64 KiB) once with an ample `limit`, search larger ones with one contextual `grep_files` call, never reconstruct a file through sequential or overlapping read windows, and stop once diagnostic evidence suffices. Both constants are appended by every harness surface that exposes output persistence and filesystem tools (bashkit, sandbox integrations, the FileSystem capability).

This is the tool's responsibility — each tool calls the helpers before constructing `ToolExecutionResult`. See `crates/core/src/tool_output_sanitizer.rs` for the primitives.

### Structured Exec Result Contract

Human-facing exec tools should return a structured result instead of a single combined `output` string. This keeps shell rendering, previews, persistence, and narration aligned across providers.

**Required fields:**

| Field | Type | Description |
|------|------|-------------|
| `stdout` | string | Sanitized stdout after verbosity truncation |
| `stderr` | string | Sanitized stderr after verbosity truncation |
| `exit_code` | integer | Process exit status |
| `success` | boolean | `true` when `exit_code == 0` |

**Recommended metadata:**

| Field | Type | Description |
|------|------|-------------|
| `cwd` | string | Effective working directory shown as secondary metadata |
| `hint` | string | Short diagnostic hint for signal exits or common recovery advice |
| `truncated` | boolean | Whether stdout or stderr was truncated for the inline result |
| `total_lines` | integer | Total stdout line count before truncation |
| `output_files` | string[] | Model-visible, file-tool-readable paths containing full persisted output |
| `full_output` | string | Model-visible, file-tool-readable stdout path when persisted |

**Legacy compatibility:** Tools may continue to carry a combined pre-truncation string in `ToolResult.raw_output` for persistence hooks and logging, but the user-visible JSON contract should use `stdout`/`stderr`.

Implementation note: shared shaping helpers live in [`crates/core/src/exec_tool_result.rs`](../../crates/core/src/exec_tool_result.rs). New shell-like tools should use that helper or match its contract exactly.

### Exec Human Representation

Exec-tool narration and cards should read like command execution, not generic infrastructure activity.

Rules:
- Title line is command-first: ``$ cargo test``, not the sandbox id or provider name
- Infra metadata (`cwd`, sandbox label/id, container/session info) is secondary
- `stderr` is visually distinct from `stdout` in both streaming and completed states
- Exit status and duration are shown tersely beside the command
- Standalone shell-like tools should render as their own transcript rows rather than being folded into grouped platform activity

This applies to built-in `bash` and provider-backed exec tools such as `daytona_exec`, `sandbox_exec`, `e2b_exec`, `docker_exec`, and similar shell-like tools.

### Background Tool Execution

`spawn_background` is a built-in meta-tool that schedules another built-in tool to run asynchronously and returns immediately with a run handle. It is generic: the caller passes `tool` plus `args`; the target tool determines whether background execution is supported.

`spawn_background` is contributed by the `background_execution` capability, which auto-activates whenever the collected tool set contains a background-capable tool. See `knowledge/execution/background-execution.md` for the cross-cutting capability contract.

Background eligibility rules:
- The target tool must set `ToolHints.supports_background = Some(true)`.
- The target tool must implement `BackgroundExecutableTool`.
- `spawn_background` must run with a worker-side `ToolContext` that includes the current `ToolRegistry`.

Contract:
- Input: `{ "tool": "...", "args": { ... }, "title"?: "...", "signal_on_completion"?: true, "schedule"?: { "cron_expression"?: "...", "scheduled_at"?: "...", "timezone"?: "..." } }`
- Immediate result without `schedule`: `run_id`, `resource_id`, target `tool`, and artifact paths under `/.background/{run_id}/`
- Immediate result with `schedule`: `schedule_id`, schedule cadence fields (`cron_expression` or `scheduled_at`, `timezone`, `next_trigger_at`), `enabled`, and `status = "scheduled"`
- Execution happens in a detached worker task
- Progress flows through `BackgroundEventSink`, which supports:
  - one-line status updates
  - streamed output deltas
  - optional structured progress (`current`, `total`, `unit`, `label`)

Artifacts and visibility:
- Each run is registered in the session resource registry as `kind = "background_run"`.
- Live metadata includes status text, output tail, and optional progress.
- Final artifacts are persisted to session VFS:
  - `/.background/{run_id}/output.log`
  - `/.background/{run_id}/result.json`
- On completion or failure, the worker may send a synthetic session message summarizing the result and artifact paths.
- That message is delivered through `PlatformStore::send_message`. **Without a platform store there is nowhere to deliver it**, and the run finishes invisibly — the agent that spawned it never learns it ended. The tool logs a warning rather than failing, since the run itself succeeded, but a host that wires `spawn_background` and no store has a hole.
- Embedded hosts wire delivery with `everruns-local`'s `LocalPlatformStore`. A session with a live host loop (a terminal UI, an editor session) must not have a turn run underneath it, so `HostRoutedRunner` + `WakeRoutes` route the completion to the host's channel when one is registered and fall through to the inner runner's synchronous turn when nobody is watching — which is the child/subagent case. What the host does with a delivered wake (coalescing, enrichment, when to run it) stays with the host.

Scheduled monitors:
- When `schedule` is provided, `spawn_background` does not start the tool immediately.
- Instead it creates a session schedule using the existing session-scheduling infrastructure.
- When that schedule fires, the session receives a synthetic user message instructing the agent to start the requested background run with the original `tool`, `args`, `title`, and `signal_on_completion` values.

Session schedule limits (apply to both `spawn_background` with a `schedule` arg and the `create_schedule` tool, since each schedule fire dispatches a real worker turn):
- Per session: at most `MAX_ACTIVE_SCHEDULES_PER_SESSION` (5) active schedules.
- Per org: at most `RESOURCE_LIMIT_MAX_SESSION_SCHEDULES_PER_ORG` (default 100) active schedules across all of the org's sessions. Uses the `RESOURCE_LIMIT_*` env family so the SaaS wrapper sets it per plan; carried on `ResourceLimitsConfig.max_session_schedules_per_org` for discoverability. Enforced worker-side because session schedules are created on the worker, not via a server command.
- Minimum cron interval: recurring crons that fire more often than `SESSION_SCHEDULE_MIN_INTERVAL_SECONDS` (default 300s / 5 min) are rejected — the session-schedule sibling of the app channel's `SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS`. One-shot (`scheduled_at`) schedules are unaffected.
- All three are rejected at create time with a clear tool error.

V1 limitation:
- Background runs are best-effort and worker-local. They are started with `tokio::spawn` inside the worker process and are not yet durable across worker restarts.
- This is intentional for the first iteration; durable resumption can be layered later without changing the tool contract.

### Turn Completion Gate

A turn ending is not the same as the user's request being done, and a host that
auto-continues has to tell the difference: a tool-only turn stopped mid-task, a
turn that asked a question is waiting on the user, a turn whose detached
background run is still going is neither.

`crates/core/src/turn_completion.rs` holds the cheap half of that judgement — a
pure function over what the turn already reported (`success`, `stop_reason`,
response text, tool-call count, whether background work is live). It decides the
clear-cut cases and answers `Evaluate` on the one genuinely ambiguous case:
tool-using work that produced a candidate final answer, where only a semantic
check can tell. Hosts pay for that check on the small fraction of turns needing
it, not on every turn.

`ContinuationBudget` bounds what the host does next in turns, tokens, *and*
wall-clock. All three, because each alone leaks: a cheap loop exhausts turns, an
expensive one exhausts tokens, one stalled on slow calls exhausts neither.

Distinct from the `usage_limit_auto_continue` capability, which resumes after a
*provider* limit resets. This gate asks whether the work is finished.

### Tool-Call Cancellation

Dropping the act future is how a cancelled turn stops tool work, and for a tool
that only awaits inside its own future that is enough — a dropped future is
never polled again. It says nothing to work the tool *left running*: a child
process, a detached watcher task, a prompt waiting on an answer.

`ToolContext.cancellation` is the signal that reaches those. The act atom mints
a token per call and cancels it when the call ends by any means — turn
cancelled, act future dropped, or the tool simply returned — so the contract a
tool sees is "this call is over; stop what you started for it". Cloning the
token into detached work is what keeps it from outliving the call.

### Tool-Side Service Stores

`ToolContext` may carry worker-backed service stores in addition to filesystem and session metadata handles. These stores let tools perform org-scoped operations without bypassing worker authorization boundaries.

Production hosts assemble these services into one runtime-owned snapshot and
derive every per-call `ToolContext` from it. A tool with a hard dependency uses
`Tool::required_context_services`; the runtime validates the active registry
before the tool is advertised. Optional services that only enrich behavior are
not hard requirements. This keeps service-free tools valid while preventing a
context-aware tool from being model-visible when its required backend was
accidentally omitted.

Current examples:

- `storage_store` for key/value and secret storage
- `image_store` for durable image artifact persistence and lookup by `image_id`
- `provider_credential_store` for default provider credentials used by tool-side API clients

This pattern is intended for tools that must call an external API directly while still relying on the control plane for secrets, org scoping, and durable storage.

### PreToolUseHook (per-tool hooks)

`PreToolUseHook` is an async hook that runs before an individual tool is executed. For sessions that load execution capabilities the same chain runs uniformly for every tool the agent calls (built-in, MCP, or client-side); blueprint sessions resolve their tools directly and do not run these hooks. A hook can mutate the `ToolCall` (returning `Continue`) or block it (returning `Block`, which skips execution and surfaces the hook's reason as the tool error, prefixed with `blocked by pre_tool_use hook:`). Hooks chain sequentially; the first `Block` wins.

Two sources feed the chain, in this order:
1. **Capability hooks** — from active capabilities via `Capability::pre_tool_use_hooks()`. This is the seam for in-process, cross-cutting policy such as approval gating (consult an approval gate, honoring each tool's `ToolHints`).
2. **User-hook specs** — `pre_tool_use` hooks dispatched per `knowledge/runtime-resources/user-hooks.md`.

Because this runs uniformly for all tools, it is the right place to gate tools the host does not implement itself (e.g. MCP tools executed by the runtime). See `Capability::pre_tool_use_hooks` and `crates/host/src/host.rs` (`load_execution_capabilities`).

### PostToolExecHook (per-tool hooks)

`PostToolExecHook` is an async hook that runs after each individual tool execution, before ActAtom emits events. Capabilities contribute hooks via `Capability::post_tool_exec_hooks()`.

Two hook slots run in sequence:
1. **Capability hooks** (`post_tool_hooks`) — from active capabilities (e.g. `tool_output_persistence`)
2. **Final hooks** (`final_post_tool_hooks`) — always-on infrastructure (e.g. EVE-225 hard limit)

Current hooks:
- **PersistOutputHook** (`tool_output_persistence` capability; also installed as an always-on final hook): When a tool declares `persist_output: true` in hints, writes stdout to `/outputs/{tool_call_id}.stdout` and stderr to `/outputs/{tool_call_id}.stderr` in session VFS, injecting `full_output`, `total_lines`, and `output_files` into the result. It skips cleanly if no session file store is present, and skips if another hook already injected `output_files`. See `crates/builtins/src/tool_output_persistence.rs`.
- **DistillOutputHook** (`tool_output_distillation` capability): For tools that do *not* declare `persist_output` (notably MCP and `web_fetch`), produces a content-aware compact inline view of large results (array sampling, string head+tail, unified-diff summary) while self-persisting the full original to `/outputs/{tool_call_id}.stdout` and injecting the same `output_files` pointer. Runs as a capability hook (before the final hooks); the `output_files` guard above prevents double-writes with PersistOutputHook. Restores the verbatim original if persistence fails. See `crates/builtins/src/tool_output_distillation.rs` and `knowledge/execution/tool-output-distillation.md`.

Current final hooks (always-on, cannot be removed):
- **PersistOutputHook**: Persists full output for any tool that declares `persist_output: true` before hard-limit truncation, independent of whether a harness explicitly enabled the persistence capability.
- **OutputHardLimitHook** (EVE-225): Enforces a 64 KiB hard ceiling on serialized tool result text. Head-truncation with UTF-8 safety; appends an LLM-actionable suffix. Logs `tracing::warn!` with tool_name, tool_call_id, result_bytes, limit when truncating. Fires regardless of which capabilities are active. See `crates/core/src/atoms/act_hooks.rs`.

### Loop Detection (EVE-227)

The `loop_detection` capability detects repeated tool loops and injects a system warning to break the loop. It uses `MessageFilterProvider::post_load` to scan loaded messages.

**Mechanism:** `ActAtom` emits stable fingerprints on tool events:

- `tool_call_fingerprint` on `tool.started` and `tool.completed`: `sha256:` over the tool name plus normalized arguments. UI-only fields such as `human_intent` and verbosity-only `output` are excluded.
- `tool_result_fingerprint` on `tool.completed`: `sha256:` over the tool name plus normalized result/error. Common volatile fields such as durations and timestamps are excluded.

After messages are loaded, the filter scans the recent available window in reverse. If `threshold` (default 3) consecutive tool results have identical call+result fingerprints, a system message is appended warning the model to change approach. It also detects repeated `read_file` ranges for the same path within the current read cluster, so alternating between the same saved-output ranges is treated as a loop while sequential paging is allowed. If result fingerprints are unavailable, it falls back to recent agent messages and detects consecutive identical tool-call batches by normalized name+arguments.

This is intentionally a rolling-window detector: hosts are not required to read all historical events. Durable hosts can rehydrate from the last relevant events; embedded hosts such as ercode can use their in-memory/logged message history. If the visible window is too short to prove a cycle, the detector continues without blocking.

**Configuration:** `{"threshold": 5}` to change the default repeat count for identical results, identical call batches, and repeated read ranges.

See `crates/builtins/src/loop_detection.rs`.

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

### Tool Scheduling

`ActAtom` receives the whole batch of tool calls the model emitted in one turn and decides *how* to run them. The policy lives in `crates/core/src/atoms/tool_scheduler.rs`; it is driven entirely by per-tool `ToolHints`, not hardcoded per tool name.

- **Concurrent by default.** Calls with no `concurrency_class` run concurrently. Read-only tools and unannotated/MCP/dynamic tools therefore parallelize freely (permissive default).
- **Class serialization.** Calls that share a non-empty `concurrency_class` run sequentially in arrival order, so mutations to the same shared resource cannot interleave. Annotated classes today: `session_workspace` (bash, `write_file`/`edit_file`/`delete_file`), `session_sql` (`sql_execute`), `session_todos` (`write_todos`), `session_storage` (`kv_store`/`secret_store`), `session_memory` (`remember`/`forget`). Different classes run in parallel with each other.
- **Concurrency cap.** Total simultaneously-executing calls are bounded by a semaphore. Default 32, override per-process with `EVERRUNS_ACT_MAX_TOOL_CONCURRENCY`.
- **CPU offload.** `cpu_bound` tools (e.g. the in-process `bash` interpreter) are executed on their own task (`tokio::spawn`) so a synchronous burst cannot starve the cooperative polling of I/O-bound tools sharing the batch. I/O-bound tools stay cooperative; no extra task is spawned for them.
- **Override.** `ActInput.parallel_tool_calls == Some(false)` forces a strictly sequential schedule (mirrors the request's `parallel_tool_calls`). Results are always returned in the model's original call order regardless of completion order.

This logic is shared by all execution paths — the in-process runtime/host loop and the durable worker both run the same `ActAtom`. The `parallel_tool_calls` field is additive and `skip_serializing_if = "Option::is_none"`, so serialized `ActInput`s from older durable runs deserialize unchanged.

### Request-level `parallel_tool_calls` (EVE-598)

`parallel_tool_calls: Option<bool>` is a config field on harness/agent/session that merges through `AgentConfigOverlay` (overlay wins) into `RuntimeAgent`, then threads through `ReasonResult` into `ActInput`, and is serialized onto the provider request:

- **Default `None`** preserves all current behavior: no field is sent to the provider, and the act scheduler uses its class-aware concurrent schedule.
- **`Some(true)`** explicitly signals the provider that parallel tool calls are wanted. This lets an embedder opt into batching independent reads/searches instead of relying on each provider's undocumented default.
- **`Some(false)`** asks the provider to emit at most one tool call per turn *and* forces the act scheduler to serialize the batch.

This field is a lower-level escape hatch. The user-facing surface is the [`parallel_tool_calls` capability](#parallel_tool_calls-capability) below; the explicit field takes precedence over the capability when both are set.

Provider wire mapping is gated by `ChatDriver::supports_parallel_tool_calls(model)` and only emitted when the resolved preference is `Some(_)`:

- **OpenAI** (Responses, and the Chat Completions `OpenAiRequest` shape used by the OpenAI Chat driver, MAI, and Fireworks): `parallel_tool_calls` top-level boolean. `supports_parallel_tool_calls` → `true`.
- **Anthropic** (Messages): `tool_choice` with `type: "auto"` and `disable_parallel_tool_use = !value`. Sent only when the request carries tools (Anthropic rejects `tool_choice` without tools). `supports_parallel_tool_calls` → `true`.
- **OpenRouter**: wraps the Open Responses driver, so the request body inherits the Responses serialization; the OpenRouter decoration layer does not strip it. `supports_parallel_tool_calls` → `true`.
- **Gemini** and **Bedrock**: their provider APIs have no request control for parallel tool calls. `supports_parallel_tool_calls` → `false`, so the field is omitted. The preference is still honored by the act scheduler (an `avoid`/`Some(false)` preference serializes execution on every provider).

`LlmCallConfig::resolved_parallel_tool_calls(supported)` performs the gating: it returns the preference when `supported` is `true`, else `None` (omit). Each driver calls it with `self.supports_parallel_tool_calls(&config.model)` while building the request.

Durable parity: the worker builds `RuntimeAgent` from gRPC-fetched schema objects, so the proto `Agent` and `Session` messages carry `parallel_tool_calls`. The server schema→proto and worker proto→schema adapters round-trip the value, preserving the operator setting through `RuntimeAgent` → `ReasonResult` → provider in durable mode.

### `parallel_tool_calls` capability

The `parallel_tool_calls` capability is the user-facing way to set the preference above. It carries a `mode`:

- **`prefer`** (default when the capability is enabled without explicit config) → `Some(true)`.
- **`avoid`** → `Some(false)`.
- **`none`** → `None` (provider default; neutralizes an inherited preference).

The capability resolves its `mode` to a preference during capability collection (`CollectedCapabilities.parallel_tool_calls`) and applies it to `RuntimeAgent.parallel_tool_calls`. An explicit `parallel_tool_calls` field on any layer wins over the capability. The **Generic** harness and the built-in **coding** harnesses enable the capability with `mode: "prefer"`.

Because harness/agent capability config is reconstructed from IDs over gRPC (durable worker), a non-default harness/agent `mode` falls back to the default (`prefer`) in durable mode; set the mode at the session level or use the explicit `parallel_tool_calls` field to override durably. This matches other config-bearing capabilities.

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
