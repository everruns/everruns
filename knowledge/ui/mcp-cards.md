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

Agent, Harness, Capability and Knowledge collection cards give a compact,
read-only view inside an MCP Apps host. Asking to show a card opens a rendered
view; clients without app rendering retain structured data and a text fallback.
The [MCP Apps](mcp-apps.md) contract owns transport, identity and sandbox behavior.

## Presentation

Reuse the same Slate entity cards as the corresponding Everruns pages. Agent
cards show identity, status, description, harness, capabilities, channels and
invocation activity. Invocation bars use the same hourly buckets and failure
colors as the agents list. Token totals are not part of this presentation.
Entity titles link to the entity in Everruns through the host's link handler.
Harness inheritance, capability usage and knowledge processing state follow the
owning product components rather than a separate MCP-specific rendering model.

The embedded route follows host theme changes and reports its size. Entity data
and API responses are bounded; oversized responses fail explicitly. Every read
uses the entrypoint's resolved organization and the shared API policy path.

## Implementation

- [Entry tools](../../crates/server/src/api/mcp_endpoint/tool_registry.rs) and
  [resource shell](../../crates/server/src/api/mcp_endpoint/apps/workspace.rs)
  associate tool results with MCP Apps resources.
- [Entity views](../../apps/ui/src/components/mcp-app/entity-view.tsx) reuse the
  shared product cards and hooks.
- [Agent adapter](../../crates/server/src/api/mcp_endpoint/cards.rs) retains the
  existing structured agent summary and size bound for nonvisual clients.
- [Workspace integration tests](../../crates/server/tests/domain/mcp_apps_test/workspace.rs)
  cover entrypoints and scoped API access; [agent card regression tests](../../crates/server/tests/domain/mcp_endpoint_test/cards.rs)
  cover internal-ID session counts and structured results.
