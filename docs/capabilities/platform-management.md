---
title: Platform Management
description: Programmatic management of harnesses, agents, and sessions from within a running session. Agents can read, create, update, and delete platform resources via tools.
---

> This is the legacy handwritten compatibility capability. New built-in
> Platform Chat sessions use the catalog-backed [Platform capability](/capabilities/platform/),
> whose `discover`, `query`, and `execute` tools stay aligned with Everruns MCP.

| | |
|---|---|
| **ID** | `platform_management` |
| **Category** | Platform |
| **Features** | None |
| **Dependencies** | `session_file_system` |

Tools to manage Everruns entities programmatically. Read, create, update, and delete harnesses, agents, and sessions; inspect session context usage; and interact with sessions by sending messages.

## Platform Documentation

This capability mounts the Everruns platform documentation at `/workspace/docs` as a virtual read-only filesystem. The documentation is embedded at compile time from the repository `docs/` directory (markdown files only) and served from memory, no database writes per session.

Agents can browse and search the docs using standard file tools (`read_file`, `list_directory`, `grep`) and bash commands (`cat`, `ls`, `grep`) via the Bashkit Shell capability.

Key sections:
- `/workspace/docs/getting-started/`, Introduction, concepts, architecture
- `/workspace/docs/features/`, SDK, CLI, UI, events, harnesses, capabilities
- `/workspace/docs/capabilities/`, Per-capability reference
- `/workspace/docs/integrations/`, External integrations (Slack, Daytona, etc.)
- `/workspace/docs/advanced/`, Budgets, compaction, embedding, network access
- `/workspace/docs/sre/`, Environment variables, runbooks

## Tools

### `read_capabilities`

Discover available capabilities (built-in, MCP servers, skills). Returns a single capability when `id` is given, otherwise a filtered list.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `id` | string | no | Capability ID to get a single capability |
| `search` | string | no | Case-insensitive filter by name, description, category, or ID |

### `read_harnesses`

Read [harnesses](/getting-started/concepts/) by ID or list all. Returns full detail (incl. system_prompt) when `id` is given.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `id` | string | no | Harness ID to get a single harness |

### `manage_harnesses`

Harness mutations: create, update, delete, copy.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `create`, `update`, `delete`, `copy` |
| `harness_id` | string | conditional | Required for `update`, `delete`, `copy` |
| `name` | string | conditional | Required for `create` |
| `system_prompt` | string | conditional | Required for `create` |
| `description` | string | no | Optional description |
| `capabilities` | string[] | no | Capabilities to enable |

### `read_agents`

Read [agents](/getting-started/concepts/) by ID or list all. Returns full detail (incl. system_prompt) when `id` is given.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `id` | string | no | Agent ID to get a single agent |

### `manage_agents`

Agent mutations: create, update, delete.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `create`, `update`, `delete` |
| `agent_id` | string | conditional | Required for `update`, `delete` |
| `name` | string | conditional | Required for `create` |
| `system_prompt` | string | conditional | Required for `create` |
| `description` | string | no | Optional description |
| `capabilities` | string[] | no | Capabilities to enable |

### `read_sessions`

Read [sessions](/getting-started/concepts/) by ID or list/filter.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `id` | string | no | Session ID to get a single session |
| `agent_id` | string | no | Optional filter by agent |
| `limit` | integer | no | Max results for list (default: 20) |

### `manage_sessions`

Session mutations: create, delete.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `operation` | enum | yes | `create`, `delete` |
| `session_id` | string | conditional | Required for `delete` |
| `harness_id` | string | no | Harness ID for the session (defaults to Generic) |
| `agent_id` | string | no | Optional for `create` |
| `title` | string | no | Optional for `create` |

### `session_send_message`

Send a user message to a session, triggering a turn.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `session_id` | string | yes | Target session ID |
| `content` | string | yes | Message content |

### `session_read_messages`

Read messages from a session.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `session_id` | string | yes | Target session ID |
| `limit` | integer | no | Max messages (default: 10) |

### `session_read_response`

Wait for session to finish processing and return the response.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `session_id` | string | yes | Target session ID |
| `timeout_secs` | integer | no | Timeout (default: 120). Set to 0 to check status without waiting. |

### `session_context_report`

Read the latest estimated context-token breakdown for a session. Returns total estimated input tokens, optional context-window size, cumulative usage, grouped sections, and per-capability contributions where available.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `session_id` | string | yes | Session ID to inspect |

## Notes

- All tool results include `ui_link` fields for navigating to the web interface
- `read_capabilities` searches across built-in, MCP server, and skill capabilities
- Session interaction follows the same turn lifecycle as the UI

## See Also

- [Platform](/capabilities/platform/), current catalog-backed capability
- [Concepts](/getting-started/concepts/), Harness, Agent, Session model
- [Agent Skills](/capabilities/agent-skills/), skill discovery
- [API Reference](/api/), full REST API
- [Capabilities Overview](/capabilities/)
