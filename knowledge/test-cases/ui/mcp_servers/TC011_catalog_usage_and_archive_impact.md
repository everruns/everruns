---
type: Test Case
title: "TC011: MCP Catalog Usage and Archive Impact"
description: "Verify active-agent usage counts and archive impact details in the MCP catalog."
tags:
  - everruns
  - test-case
  - ui
  - mcp-servers
---
# TC011: MCP Catalog Usage and Archive Impact

## Description

Verify that the MCP catalog counts active agents once and shows their names before archive.

## Preconditions

- API server and UI are running
- The current user can view and manage MCP servers
- One active preset is attached to two active agents
- One of those agents attaches the preset under two aliases
- An archived agent also attaches the preset

## Steps

1. Navigate to Registries > MCP.
2. Find the preset in the Catalog surface.
3. Review the Used by value and its tooltip.
4. Click Archive for the preset.
5. Review the archive impact message and agent list.
6. Cancel the dialog.

## Expected Result

- The catalog shows name, host, transport and protocol era, authentication, usage, status, and actions
- Used by reports two active agents and does not double-count aliases
- The tooltip states that archived agents are excluded
- The archive dialog reports two active agents and lists their names
- Cancel closes the dialog without archiving the preset
