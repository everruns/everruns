---
type: Test Case
title: "TC007: Large MCP Catalogs - Sparse Session Context"
description: "Verify a session uses a few operations from a large catalog while native schemas, CLI help, and usage accounting remain bounded and observable."
tags:
  - everruns
  - test-case
  - agents
  - mcp
  - prompt-cache
---
# TC007: Large MCP Catalogs - Sparse Session Context

## Description

Verify a session uses a few operations from a large catalog while native schemas, CLI help, and usage accounting remain bounded and observable.

## Preconditions

- Canonical local stack, Muse 1.3 Contributor, and the controlled 2,000-tool MCP fixture.
- Equivalent disposable agents for bounded native search and CLI. Record session IDs, fixture revision, and initial event cursors.
- A harmless ticket read and a staged rollback fixture with a separate healthy production deployment.

## Test Data

| Task | Observation |
| --- | --- |
| Read a ticket without changes | Exact operation discovery, one MCP read, unchanged fixture |
| Explicitly inspect full server help | Bounded text with omissions and search guidance |
| Roll back the failed staging deployment and verify it | Only required operations, verified staging state, unchanged production |
| Activate an unrelated plugin, then continue reading | Newly searchable operation; stable baseline and deferred unselected schema |

## Steps

1. Run the read on each agent. Capture actual provider request tools, selected search results, operation events, and fixture state.
2. Ask the CLI agent to run `tools telemetry --help` with full output. Inspect the returned stdout, including the omission notice; do not infer its size from UI truncation.
3. Search for the exact ticket operation and inspect only its help. Verify it remains callable even when absent from the bounded browse window.
4. Run the staged rollback. Verify state from the fixture and count actual model calls separately from MCP operations.
5. Activate an unrelated plugin on the native agent, continue without selecting it, then search for and use its harmless operation. Compare request baseline definitions and prompt transitions before and after selection.
6. Recompute usage from generation records once per reasoning iteration. Compare against request counts, cache coverage, and the largest request; exclude turn/session rollups.

## Expected Result

- Complete discovery remains available to the runtime; the model's initial surface excludes unselected MCP schemas.
- Exact-name search returns matching authorized identities without incidental result padding. Cross-server ambiguity remains explicit.
- Root/server help stays within 64 entries and 12 KiB, remains valid UTF-8, and explicitly counts omitted entries. Leaf help supplies the selected input schema.
- Native selected schemas remain within the configured optional count/byte budgets. Plugin activation makes an operation searchable before selection without advertising its schema. Incremental prompt notes and baseline tools preserve the existing cache contract.
- Actual operation evidence and final fixture state prove the task. Provider input/cache/output are counted once; missing usage/capture is visible, and cache hits are observations rather than guarantees.

## Source Contract

[Large MCP Catalogs](../../../execution/large-mcp-catalogs.md), [Native and CLI Tool Surfaces](../../../execution/tool-surfaces.md), and [event accounting](../../../../evals/common/README.md).
