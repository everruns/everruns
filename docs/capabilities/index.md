---
title: Capabilities Overview
description: Modular functionality units that extend agent behavior with tools, system prompts, and execution features
---

Capabilities are modular units that extend what an agent can do. Each capability can contribute:

- **Tools** — callable functions the agent can invoke during conversations
- **System prompt additions** — context and instructions prepended to the agent's prompt
- **Features** — UI elements unlocked when the capability is active (e.g., Workspace tab)

Agents compose capabilities — enable only what you need.

## Capability Reference

### Core

Fundamental capabilities for file operations, command execution, web access, and session management.

| Capability | ID | Tools |
|---|---|---|
| [File System](/capabilities/file-system/) | `session_file_system` | 6 |
| [Virtual Bash](/capabilities/virtual-bash/) | `virtual_bash` | 1 |
| [Session](/capabilities/session/) | `session` | 2 |
| [Storage](/capabilities/session-storage/) | `session_storage` | 2 |
| [Web Fetch](/capabilities/web-fetch/) | `web_fetch` | 1 |
| [Daytona](/capabilities/daytona/) | `daytona` | 9 |

### Data & Productivity

Structured data, time awareness, task tracking, and scheduling.

| Capability | ID | Tools |
|---|---|---|
| [SQL Database](/capabilities/sql-database/) | `session_sql_database` | 3 |
| [Current Time](/capabilities/current-time/) | `current_time` | 1 |
| [Task Management](/capabilities/task-management/) | `stateless_todo_list` | 1 |
| [Schedules](/capabilities/session-schedules/) | `session_schedule` | 3 |

### Platform & Configuration

Agent self-management, dynamic instructions, and skill discovery.

| Capability | ID | Tools |
|---|---|---|
| [Platform Management](/capabilities/platform-management/) | `platform_management` | 5 |
| [AGENTS.md](/capabilities/agent-instructions/) | `agent_instructions` | 0 |
| [Agent Skills](/capabilities/agent-skills/) | `skills` | 2 |

### Optimization

Performance and cost optimization for LLM interactions.

| Capability | ID | Tools |
|---|---|---|
| [OpenAI Tool Search](/capabilities/openai-tool-search/) | `openai_tool_search` | 0 |

### Demo

Pre-built domain simulations for testing and demonstrations.

| Capability | ID | Tools |
|---|---|---|
| [Fake Warehouse](/capabilities/fake-warehouse/) | `fake_warehouse` | 10 |
| [Fake AWS](/capabilities/fake-aws/) | `fake_aws` | 11 |
| [Fake CRM](/capabilities/fake-crm/) | `fake_crm` | 8 |

## Quick Start

### Enable via API

```bash
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Agent",
    "system_prompt": "You are a helpful assistant.",
    "capabilities": ["session_file_system", "virtual_bash", "web_fetch"]
  }'
```

### Enable via UI

1. Navigate to the Agent detail page
2. Open the **Capabilities** section
3. Toggle capabilities on/off
4. Reorder with drag handles (order affects system prompt priority)
5. Save

### List available capabilities

```bash
curl http://localhost:9300/api/v1/capabilities
```

## Key Concepts

### Dependencies

Some capabilities depend on others. Dependencies are resolved automatically at runtime — you don't need to manually add them.

| Capability | Depends On |
|---|---|
| [Virtual Bash](/capabilities/virtual-bash/) | [File System](/capabilities/file-system/) |
| [Agent Skills](/capabilities/agent-skills/) | [File System](/capabilities/file-system/) |

### Features

Capabilities declare UI features they contribute. The session aggregates features from all active capabilities to decide which UI tabs to render.

| Feature | UI Element | Contributed By |
|---|---|---|
| `file_system` | Workspace tab | [File System](/capabilities/file-system/), [Virtual Bash](/capabilities/virtual-bash/) |
| `secrets` | Storage tab | [Storage](/capabilities/session-storage/) |
| `key_value` | Storage tab | [Storage](/capabilities/session-storage/) |
| `schedules` | Schedules tab | [Schedules](/capabilities/session-schedules/) |
| `sql_database` | Database tab | [SQL Database](/capabilities/sql-database/) |

### Ordering

Capabilities are applied in the order configured on the agent. Earlier capabilities' system prompt additions appear first. Place the most important context-setting capabilities first.

## See Also

- [Concepts](/getting-started/concepts/) — how capabilities fit into the Harness → Agent → Session model
- [API Reference](/api/) — full API documentation
- [MCP Servers](/features/capabilities/#mcp-virtual-capabilities) — external tool servers as virtual capabilities
