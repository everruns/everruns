---
title: Capabilities Overview
description: Capabilities give an agent tools, system prompt fragments, and execution features. Index of every built-in capability.
sidebar:
  order: 1
---

Capabilities are modular units that extend what an agent can do. Each capability can contribute:

- **Tools**: callable functions the agent can invoke during conversations
- **System prompt additions**: context and instructions prepended to the agent's prompt
- **Features**: UI elements unlocked when the capability is active (e.g., Workspace tab)

Agents compose capabilities; enable only what you need.

## Capability Reference

Every capability a production deployment offers, with the number of tools it
adds by default. Rows marked **dev-only** are registered only
at the `dev` deployment grade. A `FEATURE_*` variable identifies a
[rollout grade override](/sre/environment-variables/#feature-rollout-grades);
organisation enrolment also applies before assigning or using gated capabilities.

### Core

Fundamental capabilities for file operations, command execution, web access, session management, time awareness, task tracking, scheduling, and agent coordination.

| Capability | ID | Tools |
|---|---|---|
| [File System](/capabilities/file-system/) | `session_file_system` | 10 |
| [Bashkit Shell](/capabilities/bashkit-shell/) | `bashkit_shell` | 1 |
| [Host Shell](/capabilities/host-shell/) | `host_shell` | 1 (Framework-only) |
| [Session](/capabilities/session/) | `session` | 2 |
| [Storage](/capabilities/session-storage/) | `session_storage` | 2 |
| [Web Fetch](/capabilities/web-fetch/) | `web_fetch` | 1 |
| [Current Time](/capabilities/current-time/) | `current_time` | 1 |
| [Message Metadata](/capabilities/message-metadata/) | `message_metadata` | 0 |
| [Ask User](/capabilities/ask-user/) | `ask_user` | 1 |
| [Task Management](/capabilities/task-management/) | `stateless_todo_list` | 1 |
| [Schedules](/capabilities/session-schedules/) | `session_schedule` | 3 |
| [Auto-Continue After Usage Limit](/capabilities/usage-limit-auto-continue/) | `usage_limit_auto_continue` | 0 |
| [AGENTS.md](/capabilities/agent-instructions/) | `agent_instructions` | 0 |
| [Agent Skills](/capabilities/agent-skills/) | `skills` | 2; `FEATURE_SKILLS` grade |
| Channel Thread Context | `channel_context` | 0 |
| System Commands | `system_commands` | 0 |

### Orchestration

Delegating work to other sessions and running it in the background.

| Capability | ID | Tools |
|---|---|---|
| [Sub Agents](/capabilities/sub-agents/) | `subagents` | 0 (contributes the `spawn_agent` delegation target) |
| Session Tasks | `session_tasks` | 5 |
| Background Execution | `background_execution` | 1 |
| [Agent Handoff](/capabilities/agent-handoff/) | `agent_handoff` | 0 (organisation opt-in, contributes the `agent` `spawn_agent` target); `FEATURE_AGENT_DELEGATION` grade |
| [A2A Agent Delegation](/capabilities/a2a-agent-delegation/) | `a2a_agent_delegation` | 0 (organisation opt-in, contributes the `external_a2a` `spawn_agent` target); `FEATURE_AGENT_DELEGATION` grade |

### Sandboxes

Cloud and container sandbox environments for isolated code execution.

| Capability | ID | Tools |
|---|---|---|
| Managed Environment | `session_sandbox` | 6 |
| [Sandbox Fleet](/capabilities/sandbox-fleet/) | `sandbox_fleet` | 8 |
| [Daytona](/capabilities/daytona/) | `daytona` | 10 |
| [E2B](/capabilities/e2b/) | `e2b` | 6 |
| Deno Sandboxes | `deno` | 6 |
| [Container Sandbox](/capabilities/container-sandbox/) | `container_sandbox` | 8 (needs `FEATURE_CONTAINER_SANDBOX=prod`) |
| [Docker Container](/capabilities/docker/) | `docker_container` | 5 (off by default, needs `FEATURE_DOCKER_CAPABILITY` rollout grade) |

### Browser

Browser automation and web interaction capabilities.

| Capability | ID | Tools |
|---|---|---|
| [Browserless](/capabilities/browserless/) | `browserless` | 7 |
| [Computer Use](/capabilities/computer-use/) | `computer_use` | 1; `FEATURE_BROWSERLESS_COMPUTER_USE` grade |

### Data and knowledge

Structured data, knowledge retrieval, and memory.

| Capability | ID | Tools |
|---|---|---|
| [SQL Database](/capabilities/sql-database/) | `session_sql_database` | 3 |
| [Retrieval Citations](/capabilities/citation-retrieval/) | `citation_retrieval` | 0 |
| [Citation Verification](/capabilities/citation-verification/) | `citation_verification` | 0 |
| [Data Knowledge](/capabilities/data-knowledge/) | `data_knowledge` | 0 |
| [Knowledge Base](/capabilities/knowledge-base/) | `knowledge_base` | 1; `FEATURE_KNOWLEDGE` grade |
| [Knowledge Index](/capabilities/knowledge-index/) | `knowledge_index` | 0 (adds `search_index` when `indexes` is set); `FEATURE_KNOWLEDGE` grade |
| [Memory](/capabilities/memory/) | `memory` | 0; `FEATURE_MEMORY` grade |

### Media

Image generation and editing workflows.

| Capability | ID | Tools |
|---|---|---|
| [OpenAI Image Generation](/capabilities/openai-image-generation/) | `gpt_image_gen` | 2 |

### Models and provider tools

Provider-executed tools and model selection.

| Capability | ID | Tools |
|---|---|---|
| [OpenAI Server Tools](/capabilities/openai-server-tools/) | `openai_server_tools` | 0 |
| [OpenRouter Server Tools](/capabilities/openrouter-server-tools/) | `openrouter_server_tools` | 0 |
| Model Scout | `model_scout` | 0 |
| OpenRouter Workspace | `openrouter_workspace` | 2 |
| OpenAI Agents API Runtime | `openai_agents_api_runtime` | 0; `FEATURE_OPENAI_AGENTS_API` grade |

### Integrations

External-service capabilities and blueprint-backed workflows.

| Capability | ID | Tools |
|---|---|---|
| [GitHub](/capabilities/github/) | `github` | 5 (6 with `allow_pull_requests`) |
| [GitHub Scout](/capabilities/github-scout/) | `github_scout` | 0 |
| [Slack](/capabilities/slack/) | `slack` | 4 |
| Cursor | `cursor` | 9 |

### Platform

Agent self-management and platform control.

| Capability | ID | Tools |
|---|---|---|
| [Platform](/capabilities/platform/) | `platform` | 3 |

### Generative UI

Structured UI the agent renders in the chat.

| Capability | ID | Tools |
|---|---|---|
| OpenUI | `openui` | 0 |
| A2UI | `a2ui` | 0 |

### Optimization

Performance and cost optimization for LLM interactions.

| Capability | ID | Tools |
|---|---|---|
| [Infinity Context](/capabilities/infinity-context/) | `infinity_context` | 1 |
| [Context Compaction](/advanced/compaction/) | `compaction` | 0 |
| [Auto Tool Search](/capabilities/tool-search/#auto-tool-search) | `auto_tool_search` | 0 (adds `tool_search` on models without native tool search) |
| [OpenAI Tool Search](/capabilities/tool-search/#hosted-openai) | `openai_tool_search` | 0 |
| [Claude Tool Search](/capabilities/tool-search/#hosted-claude) | `claude_tool_search` | 0 |
| [Tool Search](/capabilities/tool-search/#client-side) | `tool_search` | 1 |
| [Budgeting](/capabilities/budgeting/) | `budgeting` | 1 |
| [Self-Budget](/capabilities/self-budget/) | `self_budget` | 0 |
| [Parallel Tool Calls](/capabilities/parallel-tool-calls/) | `parallel_tool_calls` | 0 |
| Native Async Tools | `native_async_tools` | 0 |
| [Prompt Caching](/capabilities/prompt-caching/) | `prompt_caching` | 0 |

### Safety

Streaming-output guardrails and runtime safety nets.

| Capability | ID | Tools |
|---|---|---|
| [Prompt Canary Guardrail](/capabilities/prompt-canary-guardrail/) | `prompt_canary_guardrail` | 0 |
| [Tool Call Repair](/capabilities/tool-call-repair/) | `tool_call_repair` | 0 |
| [Output Truncation](/capabilities/output-truncation/) | `output_truncation` | 0 |
| [Guardrails](/capabilities/guardrails/) | `guardrails` | 0 |
| [Tool Approval](/capabilities/tool-approval/) | `tool_approval` | 0 |
| Progress Guard | `progress_guard` | 0 |
| [Soft Approval](/capabilities/soft-approval/) | `soft_approval` | 3 |
| [Tool Loop Detection](/capabilities/loop-detection/) | `loop_detection` | 0 |

The [`guardrails`](/capabilities/guardrails/) capability runs config-driven
checks over model output and tool activity, blocking or logging per check.
Checks can be deterministic (regex, blocklist, tool-call patterns) or
model-backed, an `llm_judge` policy or a `moderation` decision, plus
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

Harnesses also compose capabilities that are settings rather than features:
Human Intent, BTW, Error Disclosure, and Tool Output Persistence and
Distillation. The canonical [harness levels](/features/harnesses/) and
[Platform Chat](/built-ins/harnesses/platform-chat/) harness pages describe them.

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
| [Host Shell](/capabilities/host-shell/) | [File System](/capabilities/file-system/) |
| [Platform](/capabilities/platform/) | [File System](/capabilities/file-system/) (when embedded docs are enabled) |
| [Agent Skills](/capabilities/agent-skills/) | [File System](/capabilities/file-system/) |
| [GitHub Scout](/capabilities/github-scout/) | [Sub Agents](/capabilities/sub-agents/) |
| Managed Environment | [Storage](/capabilities/session-storage/) |
| [Sandbox Fleet](/capabilities/sandbox-fleet/) | [Storage](/capabilities/session-storage/) |
| [E2B](/capabilities/e2b/) | [Storage](/capabilities/session-storage/) |
| [Daytona](/capabilities/daytona/) | [Storage](/capabilities/session-storage/) |
| Deno Sandboxes | Storage |
| [Browserless](/capabilities/browserless/) | [Storage](/capabilities/session-storage/) |
| [Computer Use](/capabilities/computer-use/) | [Storage](/capabilities/session-storage/) |
| [OpenAI Image Generation](/capabilities/openai-image-generation/) | [File System](/capabilities/file-system/) |
| [Data Knowledge](/capabilities/data-knowledge/) | [File System](/capabilities/file-system/) |
| [Memory](/capabilities/memory/) | [File System](/capabilities/file-system/) |
| [Container Sandbox](/capabilities/container-sandbox/) | [Storage](/capabilities/session-storage/) |
| [A2A Agent Delegation](/capabilities/a2a-agent-delegation/) | Session Tasks |

### Features

Capabilities declare UI features they contribute. The session aggregates features from all active capabilities to decide which UI tabs to render.

| Feature | UI Element | Contributed By |
|---|---|---|
| `file_system` | Workspace tab | [File System](/capabilities/file-system/), [Bashkit Shell](/capabilities/bashkit-shell/), [Host Shell](/capabilities/host-shell/) |
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
