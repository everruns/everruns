---
type: Specification
title: "MCP Entity Cards"
description: "Read-only entity cards served through the standard MCP Apps template/data contract."
tags:
  - everruns
  - ui
  - mcp
---
# MCP Entity Cards

## Intent

An agent card gives a person a compact view of an agent's identity, status,
description, tags, usage, and session count inside an MCP Apps host. Asking to
show a card must open a rendered view rather than expose HTML source. Clients
without app rendering still receive a useful text summary.

The transport, host negotiation, and sandbox contract belong to
[MCP Apps](mcp-apps.md). Cards use that same static-template and structured-data
path. The former embedded `text/html` resource format is retired: an embedded
resource alone does not associate a tool with an MCP Apps view.

## Presentation and boundaries

Cards are read-only. They summarize existing agent information rather than
introduce a second agent configuration API. Agent lookup and session counts
remain scoped to the resolved organization through the normal MCP policy path;
see [MCP organization overrides](../integrations/mcp.md#per-call-organization_id-override).

Entity-controlled strings render as text nodes, never HTML. Avatars are inline
PNG data, with no external image fetch. The template follows host theme changes
and reports its size through the MCP Apps bridge. Serialized card data is
bounded; oversized cards fail explicitly instead of truncating information.

## Implementation and evidence

- [Card adapter and response](../../crates/server/src/api/mcp_endpoint/cards.rs)
  own presentation data and its size bound.
- [MCP Apps templates and metadata](../../crates/server/src/api/mcp_endpoint/apps/mod.rs)
  associate the tool with a readable app resource.
- [Shared view](../../crates/server/src/api/mcp_endpoint/apps/app.html)
  renders the card through safe DOM operations.
- [MCP endpoint tests](../../crates/server/tests/domain/mcp_endpoint_test.rs)
  cover the structured response and session counts keyed by internal agent ID.
- [MCP Apps integration tests](../../crates/server/tests/domain/mcp_apps_test.rs)
  cover tool metadata, template discovery, and resource reads.

The [coding-agent plugin](https://github.com/everruns/plugins) distributes the
[MCP workflow skill](../../skills/everruns-mcp/SKILL.md), including when to show
an agent card.
