---
title: Platform Management
description: Programmatic management of harnesses, agents, and sessions from within a running session. Agents can create, list, update, and delete platform resources via tools.
---

| | |
|---|---|
| **ID** | `platform_management` |
| **Category** | Platform |
| **Features** | None |
| **Dependencies** | None |

Tools to manage Everruns entities programmatically. Create, list, update, and delete harnesses, agents, and sessions — and interact with sessions by sending messages.

## Tools

### `list_capabilities`

Discover available capabilities (built-in, MCP servers, skills).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `search` | string | no | Case-insensitive filter by name, description, category, or ID |

### `manage_harnesses`

CRUD operations on [harnesses](/getting-started/concepts/).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `list`, `get`, `create`, `update`, `delete`, `copy` |
| `harness_id` | string | conditional | Required for `get`, `update`, `delete`, `copy` |
| `name` | string | conditional | Required for `create` |
| `system_prompt` | string | conditional | Required for `create` |
| `description` | string | no | Optional description |
| `capabilities` | string[] | no | Capabilities to enable |

### `manage_agents`

CRUD operations on [agents](/getting-started/concepts/).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `list`, `get`, `create`, `update`, `delete` |
| `agent_id` | string | conditional | Required for `get`, `update`, `delete` |
| `name` | string | conditional | Required for `create` |
| `system_prompt` | string | conditional | Required for `create` |
| `description` | string | no | Optional description |
| `capabilities` | string[] | no | Capabilities to enable |

### `manage_sessions`

CRUD operations on [sessions](/getting-started/concepts/).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `list`, `get`, `create`, `delete` |
| `session_id` | string | conditional | Required for `get`, `delete` |
| `harness_id` | string | conditional | Required for `create` |
| `agent_id` | string | no | Optional for `create` and `list` |
| `title` | string | no | Optional for `create` |
| `limit` | integer | no | Optional for `list` |

### `session_interact`

Send messages to sessions and manage turn lifecycle.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `send_message`, `get_messages`, `wait_for_idle` |
| `session_id` | string | yes | Target session ID |
| `content` | string | conditional | Required for `send_message` |
| `limit` | integer | no | Message count for `get_messages` (default: 10) |
| `timeout_secs` | integer | no | Timeout for `wait_for_idle` (default: 300) |

## Use Cases

- **Meta-agents** — an agent that creates and configures other agents
- **Orchestration** — spawn sessions across multiple agents and collect results
- **Self-configuration** — agent inspects available capabilities before setting up new agents
- **Automated testing** — programmatically create sessions and verify agent responses

## Example

Agent creates a specialized sub-agent and interacts with it:

```
Agent:
  → list_capabilities({ search: "file" })
  ← [{ id: "session_file_system", name: "File System", ... }]

  → manage_agents({
      operation: "create",
      name: "Code Reviewer",
      system_prompt: "You review code for bugs and style issues.",
      capabilities: ["session_file_system", "virtual_bash"]
    })
  ← { "agent_id": "agt_01xyz...", "ui_link": "/agents/agt_01xyz..." }

  → manage_sessions({
      operation: "create",
      harness_id: "hrs_01abc...",
      agent_id: "agt_01xyz..."
    })
  ← { "session_id": "ses_01def...", "ui_link": "/chat?session=ses_01def..." }

  → session_interact({
      operation: "send_message",
      session_id: "ses_01def...",
      content: "Review this Python function: def add(a, b): return a + b"
    })
```

## Notes

- All tool results include `ui_link` fields for navigating to the web interface
- `list_capabilities` searches across built-in, MCP server, and skill capabilities
- Session interaction follows the same turn lifecycle as the UI

## See Also

- [Concepts](/getting-started/concepts/) — Harness, Agent, Session model
- [Agent Skills](/capabilities/agent-skills/) — skill discovery
- [API Reference](/api/) — full REST API
- [Capabilities Overview](/capabilities/)
