---
title: Task Management
description: Structured task lists for tracking multi-step work progress
---

| | |
|---|---|
| **ID** | `stateless_todo_list` |
| **Category** | Productivity |
| **Features** | None |
| **Dependencies** | None |

Enables agents to create and manage structured task lists. State is maintained in conversation history — each tool call sends the complete list.

## Tools

### `write_todos`

Create or update the complete task list. Each call replaces the entire list.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `todos` | array | yes | Array of `{ content, status, activeForm }` objects |

Task statuses: `pending`, `in_progress`, `completed`.

## Use Cases

- **Multi-step implementations** — track progress across a complex coding task
- **User-requested lists** — manage a checklist of items the user provides
- **Workflow tracking** — show progress on multi-file refactoring or migration tasks

## Example

```
User: Refactor the auth module to use JWT

Agent:
  → write_todos([
      { content: "Analyze current auth implementation", status: "in_progress", activeForm: "Analyzing current auth" },
      { content: "Add JWT dependency", status: "pending", activeForm: "Adding JWT dependency" },
      { content: "Implement token generation", status: "pending", activeForm: "Implementing token generation" },
      { content: "Update middleware", status: "pending", activeForm: "Updating middleware" },
      { content: "Run tests", status: "pending", activeForm: "Running tests" }
    ])
```

## Notes

- **Stateless** — no database table; state lives in conversation history
- Each `write_todos` call must include the **complete** list (not incremental updates)
- Best practice: exactly one task `in_progress` at a time
- Only mark a task `completed` when fully done (tests pass, no errors)
- `activeForm` is the present-continuous label shown during execution (e.g., "Running tests")

## See Also

- [Session](/capabilities/session/) — session metadata management
- [Capabilities Overview](/capabilities/)
