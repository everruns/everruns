---
title: Task Management
description: Structured task lists for tracking multi-step work progress. Agents can create, update, and complete tasks to organize complex workflows within a session.
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

## Notes

- **Stateless**: no database table; state lives in conversation history
- Each `write_todos` call must include the **complete** list (not incremental updates)
- Best practice: exactly one task `in_progress` at a time
- Only mark a task `completed` when fully done (tests pass, no errors)
- `activeForm` is the present-continuous label shown during execution (e.g., "Running tests")

## See Also

- [Session](/capabilities/session/), session metadata management
- [Capabilities Overview](/capabilities/)
