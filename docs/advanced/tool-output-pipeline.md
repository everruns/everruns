---
title: Tool Output Pipeline
description: How Everruns processes tool results from execution to the model, verbosity budgets, distillation, persistence, hard limits, and where the full output lives
sidebar:
  order: 11
---

Agents generate most of their context from **tool output**: shell commands, file reads, web fetches, SQL queries, and MCP tool calls. Left unmanaged, a single verbose command (`git diff`, a 10,000-row query, an installer log) can blow past the model's context window, drive up cost, and bury the signal the agent actually needs.

Everruns runs every tool result through a **multi-stage pipeline** that shrinks what the model sees while keeping the full original recoverable. This page explains each stage, the order they run in, and, crucially, *where the full output goes* so the agent can get it back.

## The big picture

```
tool runs
  │
  ▼
raw output ──────────────────────────────────────────────┐
  │                                                        │  (full, lossless)
  ▼                                                        │
1. Verbosity budget        (exec/sandbox tools only)       │
  │   auto/concise/normal/verbose/full                     │
  ▼                                                        │
2. Capability hooks                                        │
  │   • Tool Output Distillation  (non-exec tools)         │
  ▼                                                        ▼
3. Final infrastructure hooks                      <display-root>/outputs/
  │   • Persist Output  (exec tools → VFS)         {tool_call_id}.stdout
  │   • Output Hard Limit  (64 KiB ceiling)        {tool_call_id}.stderr
  ▼                                                  ▲
inline result → stored in the session,               │
  shown to the model next turn ──────────────────────┘
        (read_file recovers the full original)
```

Two ideas run through the whole pipeline:

1. **Storage stays lossless.** Whatever the model sees inline, the full output is written to the session filesystem (the *destination*). The inline view always carries a pointer back to it.
2. **Each stage shrinks, none deletes.** Truncation, distillation, and masking only change the *view*. The agent can always `read_file` the persisted original.

## Stage 1, Verbosity budget (exec tools)

Exec and sandbox tools (`bash`, `*_exec`, sandboxed shells) clean their output (strip ANSI, collapse carriage returns) and apply a **verbosity budget** before returning. The mode is configurable per call; the default is `auto`:

- **Success (`exit_code == 0`)** → collapse to a compact summary (~512 bytes), because the full log is persisted (Stage 3) and the agent rarely needs it inline.
- **Failure (non-zero exit)** → keep a larger diagnostic window (~8 KiB) so the error stays debuggable in-loop.

The full pre-truncation output is stashed on the result as `raw_output` for the persistence hook to consume. Non-exec tools (MCP, web fetch, client tools) do **not** have a verbosity budget, that gap is what Stage 2 exists for.

## Stage 2, Tool Output Distillation (non-exec tools)

[Tool Output Distillation](/capabilities/) targets the tools Stage 1 doesn't: **MCP tools and `web_fetch`**, whose results otherwise enter history verbatim. It runs as a capability hook, so it executes *before* the final hooks.

For a large non-exec result, distillation produces a compact, **content-aware** inline view:

| Output shape | What you get inline |
|--------------|---------------------|
| Large JSON array | Schema-preserving sample: the first few rows + `[… N more items elided …]` |
| Long string | Head + tail window (both ends preserved), with a byte-elision marker |
| Unified diff | A diffstat-style summary: file + hunk headers and `+added / -removed` counts |
| Nested object | Each oversized field distilled; small fields untouched |

Before it replaces anything, distillation **persists the full original** to the session filesystem (same destination as Stage 3) and injects a recovery pointer. If persistence fails, or the session has no filesystem, it restores the verbatim output rather than leave a lossy result the agent can't recover. **Reversibility is never sacrificed.**

Distillation is on by default in the **generic harness**. Every transform is deterministic, so identical output distills identically and the model provider's prompt cache keeps hitting across turns.

## Stage 3, Persistence and the hard limit

Two infrastructure hooks always run last, in order:

1. **Persist Output**: for tools that declare the `persist_output` hint (exec/sandbox), writes the full `raw_output` to the session VFS and annotates the inline result with a pointer. It **skips** if a result already carries `output_files` (e.g. distillation already persisted it), so the two never double-write.
2. **Output Hard Limit**: a final, unremovable 64 KiB ceiling. By the time it runs, the result has usually already been budgeted or distilled, so it rarely fires; it's a backstop against pathological cases.

## The destination, where full output lives

Everything the pipeline elides is recoverable from the **session filesystem**:

```
<display-root>/outputs/{tool_call_id}.stdout    ← full standard output
<display-root>/outputs/{tool_call_id}.stderr    ← full standard error (when present)
```

The inline result carries the pointer in `output_files` and `full_output`, plus a human-readable note telling the model to use `read_file` (with `offset`/`limit`) when it needs detail it can't see inline. Persisted streams are capped at 1 MiB each. Deleting the session cascades and removes them.

This is the key to aggressive shrinking: because the original is one `read_file` away, the inline view can be small without the agent losing the ability to drill in.

## How this relates to compaction

The pipeline above operates on **individual tool results at capture time**. [Context Compaction](/advanced/compaction/) operates **later**, across the whole conversation, when it approaches the context window, masking or summarizing older messages at serialization time.

They compose cleanly:

- The pipeline keeps each result lean as it's produced.
- Compaction further masks older results when the *accumulated* history grows too large.
- [Infinity Context](/capabilities/) adds a `query_history` tool to retrieve older *messages* that scrolled out of the window.

Together: the pipeline controls per-result size, compaction controls total-history size, and both keep the full record recoverable.

## Summary

| Stage | Applies to | Effect | Destination of full output |
|-------|-----------|--------|----------------------------|
| Verbosity budget | exec/sandbox | Compact summary on success, diagnostic window on failure | `raw_output` → persisted in Stage 3 |
| Distillation | MCP / web fetch / non-exec | Content-aware compact view | `<display-root>/outputs/{id}.stdout` |
| Persist Output | `persist_output` tools | Lossless write + pointer | `<display-root>/outputs/{id}.{stdout,stderr}` |
| Output Hard Limit | all | 64 KiB ceiling backstop | (already persisted) |

The agent always sees a lean view and can always recover the full original with `read_file`.
