---
type: Test Case
title: "TC002: Declarative capability lifecycle"
description: "Verify that a declarative capability can bundle MCP servers, skills, and files, preserve its immutable name during editing, and be archived."
tags:
  - everruns
  - test-case
  - ui
  - capabilities
---
# TC002: Declarative capability lifecycle

## Description

Verify that a declarative capability can bundle MCP servers, skills, and files, preserve its immutable name during editing, and be archived.

## Preconditions

- The full stack is running and the user is signed in.
- One MCP server and one active skill exist.

## Test Data

| Field | Value |
|---|---|
| Create route | `/capabilities/declarative/new` |
| Unique name | `eve_995_research_pack` |
| Display name | `EVE-995 Research Pack` |
| File | `/workspace/instructions.txt` |

## Steps

1. Open `/capabilities/declarative/new` and confirm **Create** is disabled while name or description is empty.
2. Enter the unique name, display name, description, prompt addition, and `medium` risk.
3. Open **MCP Servers**, add the test server, and configure its exposed tools.
4. Open **Skills**, add the test skill.
5. Open **Files**, add the test file with recognizable content.
6. Confirm tab counters and the Summary rail show one MCP server, skill, and file, then create the capability.
7. On `/capabilities`, verify the new declarative row appears and can be found by display name and capability ID.
8. Click **Edit** and confirm `/capabilities/declarative/{declarativeId}` restores every field and bundled resource.
9. Confirm the unique name is disabled, update the display name, prompt, risk, and file content, and save.
10. Reopen the editor and verify the updates persisted while the reference remains `declarative:eve_995_research_pack`.
11. Return to `/capabilities`, archive the declarative capability, and confirm it leaves the active declarative list.

## Expected Result

- Required name and description prevent incomplete creation.
- MCP server, skill, file, prompt, and risk configuration persist across create and edit.
- The unique name cannot change after creation and remains the stable declarative reference.
- Search finds the declarative capability by display name or ID.
- Archive removes it from the active list.
