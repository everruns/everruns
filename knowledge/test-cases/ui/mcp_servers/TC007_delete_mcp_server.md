---
type: Test Case
title: "TC007: Delete Archived MCP Server"
description: "Verify archive-before-delete enforcement, cancellation, dangerous permission checks, and deleted tombstones for existing MCP references."
tags:
  - everruns
  - test-case
  - ui
  - mcp-servers
---
# TC007: Delete Archived MCP Server

## Description

Verify archive-before-delete enforcement, cancellation, dangerous permission checks, and deleted tombstones for existing MCP references.

## Preconditions

- Canonical stack; owner account satisfies `mcp_server.manage` and `mcp_server.dangerous`.
- A disposable active MCP preset is attached to an agent under a known alias.
- An admin/member fixture can manage MCP presets but lacks dangerous permission.

## Test Data

| Field | Value |
| --- | --- |
| Preset | `delete-mcp-<run-id>` |
| Existing attachment | One disposable agent and known alias |

## Steps

1. Open Settings > Organization > MCP catalog. Confirm the active preset offers Archive and no permanent Delete action.
2. As the owner, call `POST /api/v1/mcp-servers/<id>/delete` while it is active; inspect the result and re-read the preset.
3. Archive it, switch to Archived, and choose Delete. Cancel; reload and verify it remains archived.
4. Sign in as the restricted fixture. Confirm permanent Delete is unavailable and a direct delete request is rejected.
5. Return to the owner, delete the archived preset, and confirm.
6. Reload the catalog and the agent's existing attachment; attempt to fetch the destroyed preset directly.

## Expected Result

- Active destruction returns 400 and leaves the preset active.
- Cancel has no side effect; the restricted caller receives 403 and leaves the preset archived.
- Confirmed authorized destruction returns 204; the preset disappears from the catalog and direct fetch returns 404.
- Existing references render as deleted tombstones rather than disappearing or becoming a new preset.
- The deleted preset cannot supply tools; unrelated attachments remain usable.
