---
type: Test Case
title: "TC001: Import, compare and export agent packages"
description: "Checks legacy Markdown, complete ZIP assets, destination diffs and invalid input."
tags:
  - everruns
  - test-case
  - ui
  - agent-packages
---
# TC001: Import, compare and export agent packages

## Description

Checks legacy Markdown, complete ZIP assets, destination diffs and invalid input.

## Preconditions

- Canonical development stack is running with `AUTH_MODE=none`.
- Skills are enabled when testing the complete triage folder.
- Use the [package examples](../../../../examples/agents/README.md).

## Steps

1. Open Agents, select the triage ZIP and wait for validation. Review instructions, model/harness defaults, files and permissions, skill names, MCP servers and channel intent.
2. Confirm create is the default destination; import and inspect the Instructions pane. Repeat with the legacy Dad Jokes Markdown fixture.
3. Export Markdown and a complete ZIP, and verify both can be imported again.
4. Change the instructions in a copy, import it, and select the original agent as the destination.
5. Inspect Current / Imported instructions and Added / Removed / Updated file changes before applying; confirm changed instructions persist after reload.
6. Import a malformed definition and confirm a useful diagnostic appears with Import disabled.
7. Import a ZIP of the triage folder and inspect its initial files, including the skill script.
8. Compare that exported ZIP with its agent and confirm no semantic changes.

## Expected Result

- Validation and dependency errors keep the dialog open and prevent import.
- The candidate used for comparison is the candidate applied.
- Simple Markdown remains supported; ZIP preserves complete skills and binary files.
- Exports contain stable names and Instructions, with no deployment resource IDs or credentials.
- Declared channels default to disabled; explicit enablement creates drafts, not public ingress.

## Root-relative file contract

After importing the triage ZIP, open Agent → Files. Verify the tree contains
`runbook.md`, `data/example.csv`, and `.agents/skills/investigate/`, with no
`files/` wrapper. The Path input shows `runbook.md`. Export the agent as ZIP;
verify the same relative paths and `files` declarations in `agent.toml`.
Create a session and open Files. Verify the same tree and relative selected-file
path. Updating the agent must not rewrite that session's files. Attaching a new
session to its existing file tree must preserve current contents; request-level
starting files on attachment must fail before a session is created.

A valid definition with a missing destination dependency must still show its
authored preview, display the dependency diagnostic, and keep Import disabled.
