---
type: Specification
title: "Tool Output Distillation"
description: "Content-aware distillation of large non-exec tool results at capture time."
tags:
  - everruns
  - execution
---
# Tool Output Distillation

## Abstract

Large tool results bloat the context window, raise cost, and expand the prompt-injection surface. Exec/sandbox tools already get a verbosity budget (`tool_output_sanitizer`) plus lossless persistence (`tool_output_persistence`). Tools that do **not** declare the `persist_output` hint — most notably MCP tools and `web_fetch` — get neither: their output enters history verbatim and is only capped by the 64 KiB hard-limit hook, which head-truncates and discards the tail.

The `tool_output_distillation` capability closes that gap. It produces a compact, **content-aware inline view** of large non-exec tool results at capture time, while persisting the full original to the session filesystem so the agent can recover it losslessly via `read_file`.

This is the Everruns adoption of the "shell/tool output rewriting" idea (cf. RTK / headroom's ContentRouter), reframed for a runtime that owns the agent: a capability-contributed `PostToolExecHook` rather than an external CLI wrapper.

## Design Principles

1. **Target the gap, not the whole pipeline.** Exec/sandbox tools (`persist_output: true`) are already handled by budget + persistence; distillation skips them. It targets non-exec tools (MCP, `web_fetch`, client tools) whose output is otherwise unmanaged.
2. **Reversibility is mandatory.** Never replace content with a lossy view unless the full original was successfully persisted. If persistence fails (or no file store exists), restore the verbatim original. Lossy-but-irrecoverable is never allowed.
3. **Route by content shape, not tool name.** MCP tool names are arbitrary and namespaced, so the name allowlists used by compaction masking do not generalize. Distillation sniffs the JSON value.
4. **Deterministic.** Identical output distills identically, preserving provider KV-cache reuse across turns.
5. **Compose with persistence, don't duplicate it.** Reuse `tool_output_persistence::{persist_output, annotate_truncated_output}` and inject the same `output_files` pointer. `PersistOutputHook` skips when `output_files` is present, so the two never double-write.

## Pipeline Position

Per-tool output flows through these stages (see `knowledge/execution/tool-execution.md`):

```
tool returns result
  → tool-side verbosity budget (exec tools only; tool_output_sanitizer)
  → capability PostToolExecHooks   ← DistillOutputHook runs HERE
  → final hooks: PersistOutputHook → OutputHardLimitHook (64 KiB ceiling)
```

`DistillOutputHook` is a **capability** hook, so it runs **before** the final infrastructure hooks. For non-exec tools, `PersistOutputHook` does not fire (no `persist_output` hint), so distillation self-persists and the hard-limit hook only ever sees the already-distilled (smaller) result.

Both the in-process host and the durable worker assemble hooks through the same `RuntimeHostAdapter`-generic path (`crates/host/src/host.rs::execute_act_activity` → `load_execution_capabilities`), so the hook runs identically in embedded and durable execution.

## Algorithm

`DistillOutputHook::after_exec`:

1. **Skip** if the tool declares `persist_output: true` (exec/sandbox — handled elsewhere).
2. **Skip** on error results (kept verbatim — usually small and diagnostically important).
3. **Skip** if the result already carries `output_files` (already recoverable / distilled).
4. **Bounded size gate:** serialize through a hard-capped writer before cloning or traversal. Skip if the serialized result is below `MIN_DISTILL_BYTES`; also skip if it exceeds `MAX_DISTILL_INPUT_BYTES`, leaving the always-on hard-limit hook to cap the inline output without this hook doing extra large-output work.
5. **Require a session file store** (reversibility); skip if absent.
6. Clone the original, then **distill the value in place**. If the shape walker changes nothing (a large result made entirely of sub-threshold fields), **fall back** to a head+tail window over the bounded serialized value — so a large result is always bounded, persisted, and recoverable rather than risking an irrecoverable head-truncation by the 64 KiB hard-limit hook.
7. **Persist the full original** using the bounded JSON string already created for the size gate. On failure, restore the verbatim original and return.
8. **Inject the recovery pointer** (`output_files`, `full_output`, `distilled`, `distill_note`).

### Content-shape routing (`distill_value`)

Recursive, bounded by `MAX_DEPTH` and `MAX_NODES`:

| Shape | Transform |
|-------|-----------|
| Long string (> `MAX_FIELD_BYTES`) | `distill_text`: unified-diff → diffstat summary (file/hunk headers + `+a/-r` line counts); otherwise head+tail window with a byte-elision marker (preserves both ends, unlike head truncation). |
| Large array (len > `SAMPLE_ROWS`) | Keep the first `SAMPLE_ROWS` elements (each recursively distilled) + an elision marker `[… N more item(s) elided …]`. The walker never serializes each array in full just to decide whether to sample it. |
| Object | Recurse into each field; small fields untouched. |
| Scalar / small value | Unchanged. |

Constants (no per-agent config in v1): `MIN_DISTILL_BYTES = 8 KiB`, `MAX_FIELD_BYTES = 2 KiB`, `SAMPLE_ROWS = 5`, `MAX_DEPTH = 8`, `MAX_NODES = 100_000`, `MAX_DISTILL_INPUT_BYTES = 1 MiB`. See `crates/builtins/src/tool_output_distillation.rs`.

## Recovery Contract

The full original is written to `/outputs/{tool_call_id}.stdout` in the session VFS (same convention and helper as `tool_output_persistence`). Model-visible `output_files`, `full_output`, and `distill_note` pointers are rendered through `SessionFileSystem::display_path`, so mounted agent contexts expose `/workspace/...` without revealing a real-disk backing root. Every emitted pointer is accepted directly by `read_file`. This is the reversibility headroom builds via a separate CCR store; Everruns reuses the session VFS it already has.

## Configuration

Distillation is a capability with **no per-agent config in v1** — `PostToolExecHook` has no config-bearing channel (unlike `tool_definition_hooks_with_config`), matching the sibling `tool_output_persistence`. Tunable thresholds are a noted follow-up (would require a config-bearing hook variant). It depends on `session_file_system`.

It is included by default in the **generic harness** (`crates/server/src/harnesses/generic.rs`), alongside `tool_output_persistence` and `compaction`.

## Security

- **TM-DOS** — initial serialization is capped before cloning/traversal, traversal is bounded by `MAX_DEPTH` / `MAX_NODES`, arrays are sampled without full per-array serialization, and persisted size is capped at 1 MiB by `persist_output`. Distillation only ever shrinks the in-context payload it processes; oversized inputs fall through to the final hard-limit hook.
- **TM-FS** — VFS writes reuse `tool_output_persistence::persist_output`, which sanitizes the tool-call id (no path traversal) and is session-scoped.
- **TM-TOOL / prompt injection** — distillation reduces content; it never executes tool output. Markers and notes are static strings. The result remains untrusted tool output, governed by the same instruction hierarchy as before.

## Relationship to Other Capabilities

- **`tool_output_persistence`** — sibling; persists exec output before truncation. Distillation reuses its helpers and the `output_files` guard prevents double-writes. Distillation covers the non-exec tools persistence does not.
- **`compaction`** — operates later, at the model-view / serialization layer over the whole history. Distillation shrinks individual results at capture; compaction can still mask older results. Complementary, no conflict.
- **`infinity_context`** — `query_history` retrieves excluded *messages*; distillation's pointer retrieves a specific tool's *full output*. Complementary.

## References

- `knowledge/execution/tool-execution.md` — output budgets, `PersistOutputHook`, hook ordering
- `knowledge/execution/capabilities.md` — capability system
- `crates/builtins/src/tool_output_distillation.rs` — implementation
- `crates/builtins/src/tool_output_persistence.rs` — reused persistence helpers
- `docs/advanced/tool-output-pipeline.md` — end-to-end pipeline and destinations
