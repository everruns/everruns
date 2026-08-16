---
title: Customize a harness
description: Create a custom harness that bundles your preferred capabilities, system prompt baseline, and default model, then use it as the starting point for many agents.
---

A harness is the base environment for sessions, system prompt baseline, default model, and pre-bundled capabilities. Create a custom one when you have a set of defaults you want to share across many agents.

For the design rationale, see [Why three configuration layers](/explanation/concepts/#why-three-configuration-layers-harness-agent-session).

## Create a harness via API

```bash
curl -X POST http://localhost:9300/api/v1/harnesses \
  -H "Content-Type: application/json" \
  -d '{
    "name": "research-assistant",
    "display_name": "Research Assistant",
    "description": "Harness with research capabilities",
    "system_prompt": "You are a research assistant. Cite primary sources.",
    "capabilities": [
      {"ref": "session_file_system"},
      {"ref": "web_fetch"},
      {"ref": "stateless_todo_list"}
    ]
  }'
```

Names follow `[a-z0-9]+(-[a-z0-9]+)*` (up to 64 characters, no consecutive hyphens) and are unique per organization. Use `name` in API calls; `display_name` is for the UI only.

`system_prompt` is optional. Omit it when a harness exists only to bundle capabilities or MCP servers on top of a parent, the effective prompt is then composed from the parent harness, agent, session, and capabilities. For example, a capability-only harness that inherits its prompt from `generic`:

```bash
curl -X POST http://localhost:9300/api/v1/harnesses \
  -H "Content-Type: application/json" \
  -d '{
    "name": "research-tools",
    "display_name": "Research Tools",
    "parent_harness_id": "harness_...",
    "capabilities": [
      {"ref": "web_fetch"},
      {"ref": "stateless_todo_list"}
    ]
  }'
```

## Preview before creating

```bash
curl -X POST http://localhost:9300/api/v1/harnesses/preview \
  -H "Content-Type: application/json" \
  -d '{
    "system_prompt": "You are a helpful assistant.",
    "capabilities": [
      {"ref": "session_file_system"},
      {"ref": "bashkit_shell"}
    ]
  }'
```

Preview returns the merged system prompt and the tool list. Useful when capability ordering matters or when you're not sure which capability adds which prompt fragment.

## Use the harness for sessions

```bash
# By name
curl -X POST http://localhost:9300/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "harness_name": "research-assistant",
    "agent_id": "agent_..."
  }'

# Or via the CLI
everruns sessions create --harness research-assistant --agent agent_...
```

## Inheritance

Harnesses support single-parent inheritance, a child harness can extend another, layering on extra capabilities or a longer system prompt. The merge is associative: a chain of N harnesses produces the same `RuntimeAgent` as a single pre-merged harness.

This is useful when one team owns a base harness and other teams add their own specialisation on top.

## When to use a harness vs. an agent

- **Harness**: defaults shared across *many agents* (e.g., "all our internal agents have file access, web fetch, and the company AGENTS.md").
- **Agent**: per-role configuration (system prompt, voice, role-specific tools).
- **Session**: per-conversation tweaks (extra capability for this user, narrower network policy).

Don't pack agent-specific behaviour into a harness, it makes the harness a god-object and erodes the layering benefit.

## See also

- [Harnesses feature page](/features/harnesses/)
- [Built-in harnesses](/built-ins/harnesses/base/), the shipped baselines you can extend.
- [Equip an agent with tools](/how-to/equip-agents-with-tools/), at the agent layer.
