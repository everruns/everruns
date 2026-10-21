---
type: Test Case
title: "TC003: Large MCP Catalogs - Native, CLI, and Hybrid Registry Parity"
description: "Verify native, CLI, and hybrid access discover and execute the same registered MCP operation under the same authorization and schema policy."
tags:
  - everruns
  - test-case
  - agents
  - mcp
  - tool-search
---
# TC003: Large MCP Catalogs - Native, CLI, and Hybrid Registry Parity

## Description

Verify native, CLI, and hybrid access discover and execute the same registered MCP operation under the same authorization and schema policy.

## Preconditions

- Canonical DB-backed stack with a real LLM, controlled 600-tool MCP fixture, and a harmless read operation with nested JSON input and a deterministic result.
- Create equivalent disposable agents for native `tool_search`, `tools_in_shell` `mode: "cli"`, and combined bounded search plus `tools_in_shell` `mode: "hybrid"`.
- Record each agent/session ID, actor, fixture revision, starting event cursor, and expected canonical server/tool identity. Keep the retrieval policy and task identical across runs.

## Test Data

| Surface | Configuration | Required route evidence |
| --- | --- | --- |
| Native | bounded `tool_search` | Search reveal followed by native operation call |
| CLI | `tools_in_shell` with `mode: "cli"` | `tools search`, help/schema lookup, and `tools` invocation |
| Hybrid | bounded `tool_search` plus `tools_in_shell` with `mode: "hybrid"` | One native invocation and one CLI nested invocation |

## Steps

1. Run the same read task with each agent and capture model-visible tool definitions, search/help output, session events, nested-call evidence, and provider usage metadata where available.
2. In hybrid mode, ask the agent to retrieve the same record once with the native operation and once through a `tools` command.
3. Compare the fixture audit entries for canonical identity, schema digest, arguments, authorization context, and parsed results.
4. Repeat one route after a permission denial and verify the other route is subject to the same current policy.

## Expected Result

- Native, CLI, and hybrid routes resolve to the same canonical server/tool identity; JSON fields, arrays, nested objects, and enums retain their schema meaning.
- Native calls and CLI nested calls reach the same executor and apply the same authorization, validation, approval, and cancellation policy.
- Hybrid mode retains bounded native discovery and CLI access. The model can use either route; route availability does not itself grant permission.
- Compare actual serialized prefixes and provider cache metadata where available. Do not infer cache savings from final answers or tool counts.

## Source Contract

[Native and CLI Tool Surfaces](../../../execution/tool-surfaces.md), [Large MCP Catalogs](../../../execution/large-mcp-catalogs.md), and [Tools in Shell](../../../../docs/capabilities/tools-in-shell.md).
