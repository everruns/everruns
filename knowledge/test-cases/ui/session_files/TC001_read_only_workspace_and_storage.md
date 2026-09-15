---
type: Test Case
title: "TC001: Read-only workspace and storage"
description: "Verify that the session Workspace tab reads from the attached workspace, renders files without edit controls, and exposes key-value data but never secret values."
tags:
  - everruns
  - test-case
  - ui
  - sessions
  - files
  - security
---
# TC001: Read-only workspace and storage

## Description

Verify that the session Workspace tab reads from the attached workspace, renders files without edit controls, and exposes key-value data but never secret values.

## Preconditions

- The full stack is running and the user is signed in.
- A session is attached to a workspace containing nested text and binary files.
- The session has `key_value` and `secrets` features.
- Session storage contains one recognizable key-value pair and one secret with a recognizable name and value.
- A second session has no storage features.

## Test Data

| Field | Value |
|---|---|
| Route | `/sessions/{sessionId}/files` |
| Text file | `/workspace/eve-995/readme.txt` |
| Key-value key | `eve_995_mode` |
| Secret name | `EVE_995_SECRET` |

## Steps

1. Open the populated session and select **Workspace**.
2. Confirm the initial viewer says **No file selected** and the sidebar lists the attached workspace tree.
3. Expand the nested folder, select the text file, and confirm its path and content render.
4. Select the binary file and confirm the viewer uses its supported preview or download state without corrupt text.
5. Verify there are no upload, drag-and-drop, edit, save, delete, key-value write, or secret write controls.
6. In **Session storage**, confirm the key-value row shows the key, full value, and updated time.
7. Confirm the secret row shows only the secret name and updated time; search the page and network response visible to the UI for the recognizable secret value.
8. Open the second session's Workspace tab.
9. Open a session attached to a shared workspace, if available, and confirm the file tree belongs to its `workspace_id` rather than its session ID.

## Expected Result

- The file browser reads the workspace attached to the selected session.
- Text and binary files use read-only viewing behavior.
- The tab contains no mutation controls for files or session storage.
- Key-value values are visible, while secret values are never returned or rendered.
- Session storage is absent when neither `key_value` nor `secrets` is enabled.
- Shared-workspace sessions display the canonical shared workspace contents.
