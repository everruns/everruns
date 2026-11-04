---
type: Test Case
title: "TC001: Large MCP Catalogs - Paginated Discovery and Search"
description: "Verify all MCP pages are discovered and a tool found on a later page is revealed with its complete schema and executes correctly."
tags:
  - everruns
  - test-case
  - agents
  - mcp
  - tool-search
---
# TC001: Large MCP Catalogs - Paginated Discovery and Search

## Description

Verify all MCP pages are discovered and a tool found on a later page is revealed with its complete schema and executes correctly.

## Preconditions

- Canonical DB-backed stack with a real LLM, a disposable agent, and a controlled paginated MCP fixture.
- Enable bounded `tool_search` with `max_active_tools: 32`, `max_active_bytes: 65536`, and `max_result_bytes: 32768`.
- Fixture exposes 600 tools, then 2,000 tools, over 100-tool pages. Put the target tool on the final page and record fixture revision, catalog size, page count, session ID, actor, and starting event cursor.
- Target operation is a harmless read with a nested schema and a deterministic result; fixture audit records its original server and tool name, arguments, and result.

## Test Data

| Catalog | Target | Query |
| --- | --- | --- |
| 600 tools | `get_incident` on the final page | A natural-language description that omits the exact operation name |
| 2,000 tools | `get_ticket` on the final page | A natural-language description of the needed information |

## Steps

1. Start a fresh session for each catalog size and ask the agent to retrieve the target record using only the attached MCP server.
2. Inspect the model request's initial native tool definitions. Record the schemas and their serialized byte count.
3. Inspect the search call and result, then the following model request. Verify the target's full schema is active before its native invocation.
4. Inspect the session events and fixture audit after completion. Record the actual target server, original tool name, arguments, schema revision, result, and page requests.
5. Repeat the same task in a warm session and compare catalog fetches, active schemas, prompt prefix, and provider cache metadata where available.

## Expected Result

- Discovery traverses every cursor; the target on the final page is searchable.
- The optional active native schemas respect the configured count and byte limits; eager/never-defer tools are additional. Search reveals the complete authoritative schema for the selected operation before a structured call.
- The target call reaches the original server/tool with schema-valid arguments and returns the fixture's deterministic result.
- A warm run does not turn a partial catalog into a complete one or disclose unrelated full schemas. Record cache and latency observations; this case does not assert a performance improvement.

## Source Contract

[Large MCP Catalogs](../../../execution/large-mcp-catalogs.md), [Native and CLI Tool Surfaces](../../../execution/tool-surfaces.md), and [Tool Search](../../../../docs/capabilities/tool-search.md).
