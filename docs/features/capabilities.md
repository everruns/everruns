---
title: Capabilities
description: Capabilities are the modular units that give an agent its tools, system prompt fragments, and session state. Conceptual overview with links to the full reference.
---

A **capability** is a self-contained unit that extends an agent. Each capability can contribute three kinds of thing:

1. **Tools**: functions the agent can invoke during a turn.
2. **System prompt additions**: text prepended to the agent's prompt that teaches the model when and how to use those tools.
3. **Mount points**: files, directories, or session state the tools need to operate.

Agents *compose* capabilities. Enable as many as you need; leave the rest disabled. The runtime resolves capabilities in topological order (dependencies first) and concatenates their system prompt fragments in the order you configured them on the agent.

## Why capabilities, not just tools

A bare tool registration is a function with a JSON schema. Capabilities exist because real tools need more than that. To use `bashkit_shell` effectively, the agent needs both the `bash` tool *and* the prompt fragment explaining the sandbox model *and* the session filesystem (a dependency). Capabilities bundle those concerns into one enable/disable unit.

See [Why capabilities are first-class](/explanation/concepts/#why-capabilities-are-first-class) for the full rationale.

## Where capabilities come from

| Source | ID format | Example |
|---|---|---|
| Built-in | `snake_case` | `web_fetch`, `session_file_system` |
| MCP server | `mcp:{uuid}` | `mcp:550e8400-...` |
| Registry skill | `skill:{uuid}` | `skill:550e8400-...` |
| Declarative | `declarative:{name}` | `declarative:research_pack` |

All four kinds participate in the same merge, dependency resolution, and tool-name prefixing. The agent doesn't care where a tool came from.

## Where to attach them

Capabilities can be attached at three layers, and the layers stack additively:

- **Harness**: defaults for every session that uses this harness.
- **Agent**: capabilities for this specific role.
- **Session**: extras for this one conversation.

See [Why three configuration layers](/explanation/concepts/#why-three-configuration-layers-harness-agent-session) for how the merge works.

## Browse the catalog

The full list of built-in capabilities, organised by category, lives in the reference:

- **[Capabilities reference](/capabilities/)**: every capability with ID, tools, parameters, and dependencies.

## Do something

- [Equip an agent with tools](/how-to/equip-agents-with-tools/), the practical recipe.
- [Give an agent web access](/how-to/give-an-agent-web-access/), narrower task with network policies.
- [Customize a harness](/how-to/customize-a-harness/), bundle capabilities at the harness layer.

## See also

- [Concepts](/explanation/concepts/), the entity model in full.
