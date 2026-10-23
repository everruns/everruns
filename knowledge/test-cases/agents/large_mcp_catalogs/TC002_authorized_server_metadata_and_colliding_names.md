---
type: Test Case
title: "TC002: Large MCP Catalogs - Authorized Server Metadata and Colliding Names"
description: "Verify search can use attached server descriptions and schema text while distinguishing same-named tools and excluding unauthorized catalogs."
tags:
  - everruns
  - test-case
  - agents
  - mcp
  - tool-search
---
# TC002: Large MCP Catalogs - Authorized Server Metadata and Colliding Names

## Description

Verify search can use attached server descriptions and schema text while distinguishing same-named tools and excluding unauthorized catalogs.

## Preconditions

- Canonical DB-backed stack with a real LLM and a controlled fixture containing at least two attached servers and one unattached or unauthorized server.
- Use distinct server names and descriptions. Two authorized servers expose same-named operations with incompatible, non-mutating schemas and different deterministic results; only the server description and nested field descriptions contain the task's domain language.
- Record agent, actor, session, server identities, authorization state, schema digests, event cursor, and fixture audit state. Configure bounded `tool_search`.

## Test Data

| Server | Description | Same-named operation | Distinguishing result |
| --- | --- | --- | --- |
| `alpha` | Geothermal drilling telemetry | `search` | `source: alpha` |
| `beta` | Urban water quality records | `search` | `source: beta` |
| `hidden` | Private maintenance records; not attached or authorized | `search` | Must never be returned or called |

## Steps

1. Ask for a geothermal telemetry record without naming `alpha` or `search`. Inspect returned search matches and the selected schema.
2. Ask for a water quality record with the same omission. Inspect the selected schema and fixture audit.
3. Search for a term present only in the unauthorized server description, then try an explicit lookup using its server name.
4. If supported by the configured test host, attach `hidden`, remove its authorization, and repeat search and invocation from the same session.

## Expected Result

- Server names/descriptions, operation descriptions, and schema text can contribute to discovery; matching server context guides selection without hiding other authorized servers from a cross-server request.
- Same-named operations remain server-qualified. Each successful invocation reaches the server selected by the query and uses that operation's own schema.
- Unattached or unauthorized server names, descriptions, schemas, and operations do not appear in search results or execute. Revocation blocks invocation even if prior conversation history contains a disclosed schema.
- Fixture audits and current authorization state, not the agent's final prose alone, determine the result.

## Source Contract

[Large MCP Catalogs](../../../execution/large-mcp-catalogs.md), [authorization-scoped MCP discovery](../../../execution/large-mcp-catalogs.md#canonical-catalog-and-retrieval), and [MCP](../../../integrations/mcp.md).
