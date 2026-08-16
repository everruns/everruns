---
type: Specification
title: "Background Execution Capability"
description: "`background_execution` capability and cross-cutting / auto-activation contract."
tags:
  - everruns
  - execution
---
# Background Execution Capability

The `background_execution` capability contributes the `spawn_background` meta-tool to the agent's tool set. It implements the **cross-cutting capability** pattern: a capability that activates automatically based on properties of the active tool set, rather than being explicitly requested by the agent author.

This document captures the contract. Tool-side behavior of `spawn_background` (artifacts, schedules, run handles) lives in `knowledge/execution/tool-execution.md` under "Background Tool Execution".

## Goal

Each accepted background run is tracked as a session task
(`kind = background_tool`) with progress, state, and `task.*` events, see
[`knowledge/runtime-resources/session-tasks.md`](../runtime-resources/session-tasks.md).

`spawn_background` is the generic background runner. It wraps any registered built-in tool that opts in via hints. Before EVE-501 the meta-tool was registered into the worker's tool registry by default but was never advertised to the model, sessions had `bash` with `supports_background=true` but no visible `spawn_background`, so models fell back to raw shell detaching. The fix is to expose `spawn_background` through the same capability path that contributes all other model-visible tools.

## Contract

1. **Tool contributor.** `background_execution` contributes a single tool implementation: `SpawnBackgroundTool`. It does not own a target tool; it dispatches to whatever tool the model names in the `tool` argument.

2. **Auto-activation rule.** `collect_capabilities_with_configs` activates `background_execution` automatically when **any** collected tool definition declares `ToolHints.supports_background == Some(true)`. The capability does not need to appear in the agent's configured capability list.

3. **Idempotence.** Explicit selection (id `"background_execution"` in the agent's capabilities) and auto-activation must not produce duplicate `spawn_background` entries. The auto-activator no-ops when the capability is already in `applied_ids`.

4. **Lockstep with the worker registry.** Both `tool_definitions` (model-visible) and `tools` (worker execution registry) gain `spawn_background` from the same activation event. `ToolRegistry::with_defaults()` does **not** include `SpawnBackgroundTool`, if the model cannot see the tool, the worker must not silently dispatch it from a hidden default.

5. **No owner capability.** `bashkit_shell` only advertises `supports_background=true` on the `bash` tool and implements `BackgroundExecutableTool`. It does not contribute `spawn_background`. The same rule applies to any future background-capable tool: declare the hint, implement the trait, and the meta-tool surfaces via the generic capability.

## Cross-cutting capability pattern

`background_execution` is the reference example of a capability that is contributed by hints on other capabilities rather than by explicit configuration. Future meta-tools that wrap or augment other tools (e.g. progress reporting, retries, schedulable wrappers) should follow this pattern instead of attaching to a single owner:

- Declare a stable capability id and contribute the meta-tool from `tools()`.
- Express activation as a property of the collected tool set (a specific hint, capability id, or tool name) and add the check to the auto-activator block in `collect_capabilities_with_configs`.
- Keep the meta-tool out of `ToolRegistry::with_defaults()` to preserve the lockstep invariant.
- Allow explicit selection by id for tests and overlays; the auto-activator must remain idempotent.

When a new cross-cutting capability is added, extend the auto-activation block rather than introducing a parallel activation phase, keep all rules in one place.

## Test policy

The capability ships with three layers of tests:

- **Capability unit tests** (`crates/platform/src/capabilities/background_execution.rs`): metadata, single-tool contribution, and hosted-registry activation.
- **Activation unit tests** (`crates/core/src/capabilities/mod.rs`): positive trigger via `bashkit_shell`, negative trigger with `current_time`, idempotence under explicit + auto-activation.
- **Lockstep regression** (`crates/core/src/tools.rs`, `crates/core/src/capabilities/mod.rs`): `ToolRegistry::with_defaults()` must not include `spawn_background`.

End-to-end scripted-session coverage (using `llmsim` scripted mode to drive an agent through a real `spawn_background` call) is the suggested next layer; it belongs alongside the existing `crates/server/tests/workflow_test.rs` LlmSim scenarios.

## Related specs

- `knowledge/execution/tool-execution.md`, `spawn_background` runtime contract (artifacts, schedules, signal-on-completion).
- `knowledge/execution/capabilities.md`, Capability system, registration, dependency resolution.
