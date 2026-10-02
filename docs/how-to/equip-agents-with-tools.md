---
title: Equip an agent with tools
description: Assign capabilities to an agent so it can read files, run shell commands, fetch URLs, and track tasks.
---

This guide assigns common capabilities to an agent so it can interact with files, run commands, and fetch URLs. For the full catalog see the [Capabilities reference](/capabilities/).

## Common capabilities

| Capability ID | Tools provided | What it's for |
|---|---|---|
| `web_fetch` | `web_fetch` | Fetch URLs, convert HTML to markdown |
| `session_file_system` | `read_file`, `write_file`, `edit_file`, `list_directory`, `grep_files`, `delete_file`, `stat_file` | Per-session virtual filesystem |
| `bashkit_shell` | `bash` | Sandboxed bash shell |
| `stateless_todo_list` | `write_todos` | Structured task tracking |
| `current_time` | `get_current_time` | Current date/time awareness |
| `session_storage` | `kv_store`, `secret_store` | Key/value and encrypted secrets |

## Assign capabilities at creation

```python
from everruns_sdk import AgentCapabilityConfig

agent = await client.agents.create(
    name="researcher",
    display_name="Researcher",
    system_prompt="You research topics and save notes to /workspace.",
    capabilities=[
        AgentCapabilityConfig(ref="web_fetch"),
        AgentCapabilityConfig(ref="session_file_system"),
        AgentCapabilityConfig(ref="stateless_todo_list"),
    ],
)
```

## Update an existing agent

The SDK has no partial agent update. Re-apply the full definition with `apply_by_name`, which updates the agent when the name exists:

```python
agent = await client.agents.apply_by_name(
    "researcher",
    "You research topics and save notes to /workspace.",
    capabilities=[
        AgentCapabilityConfig(ref="web_fetch"),
        AgentCapabilityConfig(ref="session_file_system"),
        AgentCapabilityConfig(ref="bashkit_shell"),
    ],
)
```

## Configure a capability

Some capabilities accept per-agent configuration. Pass it in `config`:

```python
agent = await client.agents.apply_by_name(
    "researcher",
    "You research topics and save notes to /workspace.",
    capabilities=[
        AgentCapabilityConfig(ref="web_fetch", config={"enable_file_download": True}),
        AgentCapabilityConfig(ref="session_file_system"),
    ],
)
```

## Verify the agent has the tools

```python
agent = await client.agents.get(agent.id)
for cap in agent.capabilities:
    print(cap.ref, cap.config or "")
```

## Notes on ordering

Capability order matters: capabilities earlier in the list contribute their system prompt fragments first. Put high-priority context (project conventions, AGENTS.md) before tool-specific guidance.

## See also

- [Capabilities reference](/capabilities/), all available capabilities.
- [Why capabilities are first-class](/explanation/concepts/#why-capabilities-are-first-class), the design rationale.
- [Give an agent web access](/how-to/give-an-agent-web-access/), narrower task with network policies.
