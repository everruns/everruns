---
type: Test Case
title: "TC015: MCP Tool Risk Labels"
description: "Verify manual tool labels persist, override upstream annotations, survive tool-list changes, and remain distinct from unconfirmed suggestions."
tags:
  - everruns
  - test-case
  - ui
  - mcp-servers
---
# TC015: MCP Tool Risk Labels

## Description

Verify manual tool labels persist, override upstream annotations, survive tool-list changes, and remain distinct from unconfirmed suggestions.

## Preconditions

- Canonical stack with a working controlled MCP preset exposing two harmless tools and editable upstream annotations.
- Caller can manage the preset; a view-only fixture is available through a custom permission resolver if needed.
- Decisions service configured for suggestion checks; otherwise mark that portion blocked, not passed.

## Test Data

| Tool | Label action |
| --- | --- |
| Harmless read tool | Set Read only |
| Harmless mutating fixture tool | Set Changes things; reset to Default |
| Unlabeled tool | Request a suggestion without applying it |

## Steps

1. Open the preset's Tools dialog. Set manual labels, close it, and reload.
2. Inspect a disposable agent's effective tool definitions: saved labels should take precedence over contradictory upstream annotations.
3. Request suggestions for unlabeled tools. Inspect suggestions and effective definitions before confirming anything.
4. Accept one suggestion by setting its label; reset another tool to Default and re-read definitions.
5. Make the controlled server temporarily remove, then restore a labeled tool; refresh discovery after each change and inspect its saved label.
6. As the view-only fixture, inspect the dialog and attempt the label-update API directly. Restore upstream fixtures afterward.

## Expected Result

- Manual labels persist and drive effective risk annotations. Read-only clears contradictory destructive/open-world treatment.
- A suggestion is visible but does not silently change the active label or approval behavior; only an explicit saved choice applies it.
- Default removes the manual override and restores upstream/default behavior. A returning tool retains its saved label.
- View-only cannot mutate labels; direct requests return 403 and leave saved values unchanged.

## Source Contract

[Tool labels](../../../../crates/server/src/domains/mcp_servers/tool_labels/mod.rs).
