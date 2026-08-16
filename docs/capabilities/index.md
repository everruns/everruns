---
title: Capabilities Overview
description: Modular capabilities that extend agent behavior with tools, system prompts, and execution features. Browse all built-in and custom capabilities available.
sidebar:
  order: 1
---

Capabilities are modular units that extend what an agent can do. Each capability can contribute:

- **Tools**: callable functions the agent can invoke during conversations
- **System prompt additions**: context and instructions prepended to the agent's prompt
- **Features**: UI elements unlocked when the capability is active (e.g., Workspace tab)

Agents compose capabilities, enable only what you need.

## Capability Reference

### Core

Fundamental capabilities for file operations, command execution, web access, session management, time awareness, task tracking, scheduling, and agent coordination.

| Capability | ID | Tools |
|---|---|---|
| [File System](/capabilities/file-system/) | `session_file_system` | 6 |
| [Bashkit Shell](/capabilities/bashkit-shell/) | `bashkit_shell` | 1 |
| [Session](/capabilities/session/) | `session` | 2 |
| [Storage](/capabilities/session-storage/) | `session_storage` | 2 |
| [Web Fetch](/capabilities/web-fetch/) | `web_fetch` | 1 |
| [Current Time](/capabilities/current-time/) | `current_time` | 1 |
| [Message Metadata](/capabilities/message-metadata/) | `message_metadata` | 0 |
| [Task Management](/capabilities/task-management/) | `stateless_todo_list` | 1 |
| [Schedules](/capabilities/session-schedules/) | `session_schedule` | 3 |
| [Auto-Continue After Usage Limit](/capabilities/usage-limit-auto-continue/) | `usage_limit_auto_continue` | 0 |
| [Sub Agents](/capabilities/sub-agents/) | `subagents` | 3 |
| [AGENTS.md](/capabilities/agent-instructions/) | `agent_instructions` | 0 |
| [Agent Skills](/capabilities/agent-skills/) | `skills` | 2 |

### Sandboxes

Cloud and container sandbox environments for isolated code execution.

| Capability | ID | Tools |
|---|---|---|
| [Daytona](/capabilities/daytona/) | `daytona` | 10 |
| [Deno Sandboxes](/capabilities/deno/) | `deno` | 6 |
| [E2B](/capabilities/e2b/) | `e2b` | 6 |
| [Docker Container](/capabilities/docker/) | `docker_container` | 5 |

### Browser

Browser automation and web interaction capabilities.

| Capability | ID | Tools |
|---|---|---|
| [Browserless](/capabilities/browserless/) | `browserless` | 7 |

### Data

Structured data and knowledge capabilities.

| Capability | ID | Tools |
|---|---|---|
| [SQL Database](/capabilities/sql-database/) | `session_sql_database` | 3 |
| [Retrieval Citations](/capabilities/citation-retrieval/) | `citation_retrieval` | 0 |
| [Citation Verification](/capabilities/citation-verification/) | `citation_verification` | 0 |

### Media

Image generation and editing workflows.

| Capability | ID | Tools |
|---|---|---|
| [OpenAI Image Generation](/capabilities/openai-image-generation/) | `gpt_image_gen` | 2 |

### Tools

Provider-executed and built-in tool capabilities.

| Capability | ID | Tools |
|---|---|---|
| [OpenRouter Server Tools](/capabilities/openrouter-server-tools/) | `openrouter_server_tools` | 0 |

### Integrations

External-service capabilities and blueprint-backed workflows.

| Capability | ID | Tools |
|---|---|---|
| [GitHub Scout](/capabilities/github-scout/) | `github_scout` | 0 |

### Platform

Agent self-management and platform control.

| Capability | ID | Tools |
|---|---|---|
| [Platform](/capabilities/platform/) | `platform` | 3 |
| [Platform Management (legacy)](/capabilities/platform-management/) | `platform_management` | 14 |

### Optimization

Performance and cost optimization for LLM interactions.

| Capability | ID | Tools |
|---|---|---|
| [Infinity Context](/capabilities/infinity-context/) | `infinity_context` | 1 |
| [Auto Tool Search](/capabilities/auto-tool-search/) | `auto_tool_search` | 1 |
| [OpenAI Tool Search](/capabilities/openai-tool-search/) | `openai_tool_search` | 0 |
| [Claude Tool Search](/capabilities/claude-tool-search/) | `claude_tool_search` | 0 |
| [Tool Search](/capabilities/tool-search/) | `tool_search` | 1 |
| [Budgeting](/capabilities/budgeting/) | `budgeting` | 1 |
| [Self-Budget](/capabilities/self-budget/) | `self_budget` | 0 |
| [Parallel Tool Calls](/capabilities/parallel-tool-calls/) | `parallel_tool_calls` | 0 |

### Safety

Streaming-output guardrails and runtime safety nets.

| Capability | ID | Tools |
|---|---|---|
| [Prompt Canary Guardrail](/capabilities/prompt-canary-guardrail/) | `prompt_canary_guardrail` | 0 |
| [Tool Call Repair](/capabilities/tool-call-repair/) | `tool_call_repair` | 0 |
| [Guardrails](/capabilities/guardrails/) | `guardrails` | 0 |

The [`guardrails`](/capabilities/guardrails/) capability runs config-driven
checks over model output and tool activity, blocking or logging per check.
Checks can be deterministic (regex, blocklist, tool-call patterns) or
model-backed, an `llm_judge` policy or a `moderation` classifier, plus
delegation to an external guardrail over scoped MCP. Each check binds a rule to
a stage (`output`, `tool_use`, `tool_output`) with an `on_fail` of `block` or
`log`; model-backed and MCP checks send a bounded excerpt off the sync path and
fail open. Use advisory mode and the
`POST /v1/capabilities/guardrails/dry-run` endpoint to tune against false
positives before enforcing. For ready-made starting points, list the gallery at
`GET /v1/capabilities/guardrails/examples`, each preset carries a `data_egress`
signal (`none` vs. `utility_llm`), and drop a preset's `config` into the
agent's `guardrails` capability config.

### Automation

Run shell commands at lifecycle and tool events. Block, mutate, or audit
agent actions from outside the model.

| Capability | ID | Tools |
|---|---|---|
| [User Hooks](/capabilities/user-hooks/) | `user_hooks` | 0 |

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
    "capabilities": ["session_file_system", "bashkit_shell", "web_fetch"]
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

### Create a declarative capability

Declarative capabilities are persisted capability definitions made from data:
system prompt text, scoped MCP servers, text file mounts, and skill packages.
They use a public resource ID like `cap_...` and a stable capability reference
like `declarative:research_pack`.

```bash
curl -X POST http://localhost:9300/api/v1/capabilities \
  -H "Content-Type: application/json" \
  -d '{
    "definition": {
      "name": "research_pack",
      "display_name": "Research Pack",
      "description": "Default research behavior and resources.",
      "system_prompt": "Prefer primary sources and cite them clearly.",
      "risk_level": "low"
    }
  }'
```

Agents and harnesses can use the canonical reference:

```json
{ "ref": "declarative:research_pack" }
```

For convenience, agent and harness write APIs also accept the plain unique name
when it matches a declarative capability:

```json
{ "ref": "research_pack" }
```

## Key Concepts

### Dependencies

Some capabilities depend on others. Dependencies are resolved automatically at runtime, you don't need to manually add them.

| Capability | Depends On |
|---|---|
| [Bashkit Shell](/capabilities/bashkit-shell/) | [File System](/capabilities/file-system/) |
| [Agent Skills](/capabilities/agent-skills/) | [File System](/capabilities/file-system/) |
| [GitHub Scout](/capabilities/github-scout/) | [Sub Agents](/capabilities/sub-agents/) |
| [Deno Sandboxes](/capabilities/deno/) | [Storage](/capabilities/session-storage/) |
| [E2B](/capabilities/e2b/) | [Storage](/capabilities/session-storage/) |

### Features

Capabilities declare UI features they contribute. The session aggregates features from all active capabilities to decide which UI tabs to render.

| Feature | UI Element | Contributed By |
|---|---|---|
| `file_system` | Workspace tab | [File System](/capabilities/file-system/), [Bashkit Shell](/capabilities/bashkit-shell/) |
| `secrets` | Storage tab | [Storage](/capabilities/session-storage/) |
| `key_value` | Storage tab | [Storage](/capabilities/session-storage/) |
| `schedules` | Schedules tab | [Schedules](/capabilities/session-schedules/) |
| `sql_database` | Database tab | [SQL Database](/capabilities/sql-database/) |
| `subagents` | Subagents tab | [Sub Agents](/capabilities/sub-agents/) |
| `citations` | Inline citation chips + Sources strip | [Retrieval Citations](/capabilities/citation-retrieval/), [Citation Verification](/capabilities/citation-verification/) |

### Ordering

Capabilities are applied in the order configured on the agent. Earlier capabilities' system prompt additions appear first. Place the most important context-setting capabilities first.

## See Also

- [Concepts](/getting-started/concepts/), how capabilities fit into the Harness → Agent → Session model
- [API Reference](/api/), full API documentation
- [MCP Servers](/features/mcp/), external tool servers as virtual capabilities
