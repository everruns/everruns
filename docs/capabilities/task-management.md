---
title: Task Management
description: Structured task lists for tracking multi-step work within a session.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `stateless_todo_list` |
| **Category** | Core |
| **Features** | None |
| **Dependencies** | None |

Enables agents to create and manage structured task lists. State is maintained in conversation history, each tool call sends the complete list.

## Tools

### `write_todos`

Create or update the complete task list. Each call replaces the entire list.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `todos` | array | yes | Array of `{ content, status, activeForm }` objects |

Task statuses: `pending`, `in_progress`, `completed`.

### `report_done`

Report finished work once, before the final reply: what was done, how it was checked, and links
to what it produced. The session's work report shows this summary instead of the first sentence
of the reply, and a coordinated worker's `complete_assignment` is read the same way.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `summary` | string | yes | One or two sentences, leading with the result |
| `validation` | string | no | How the work was checked |
| `outcome` | string | no | `done` (default) or `failed` |
| `artifacts` | array | no | Up to 10 `{ name, type, url?, path? }` links |

## Notes

- **Stateless**: no database table; state lives in conversation history
- Each `write_todos` call must include the **complete** list (not incremental updates)
- Best practice: exactly one task `in_progress` at a time
- Only mark a task `completed` when fully done (tests pass, no errors)
- `activeForm` is the present-continuous label shown during execution (e.g., "Running tests")

## See Also

- [Session](/capabilities/session/), session metadata management
- [Capabilities Overview](/capabilities/)
