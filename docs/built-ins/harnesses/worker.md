---
title: Worker Harness
description: The worker kit with no compute, for agents that work through tools, MCP servers and files.
---

Worker inherits [Base](/built-ins/harnesses/base/) and adds everything a working agent needs except
a shell: session files, project instructions (`AGENTS.md`), session tools, durable and distilled tool
output, skills, history retrieval, automatic tool discovery, budget awareness, todo lists, subagents
and task controls.

It runs no code. Choose it for agents that act through tools and MCP servers: triage and integration
agents over Linear, Slack or GitHub, research agents over knowledge and files, and organizations that
want no code execution at all. It needs no sandbox provider.

Worker is also the parent of the two shell workers:

- [Bashkit Worker](/built-ins/harnesses/bashkit-worker/) adds the Bashkit virtual shell.
- [Sandbox Worker](/built-ins/harnesses/sandbox-worker/) adds a full sandbox.

An Agent on Worker may still set a Sandbox policy; the selected Sandbox Template then supplies the
shell. Pick one of the shell workers when the agent always needs one.

Subagents create separate child sessions. Ordinary children inherit the parent configuration; blueprints provide specialist behavior. Depth and root-tree task limits bound delegation. Assigning this harness requires permission to assign its high-risk capabilities.

Web access, secret/KV tools, organization memory mounts and session scheduling remain opt-in. Recurring agent execution is configured with Agent Triggers.

Choose `worker` with the API's `harness_name` or the CLI's `--harness` option.

```rust
let harness = everruns::Harness::worker();
```

Framework constructors load the same capability data as the hosted presets. Enable the host integrations required by the tools you use; a preset does not compile optional integrations into your application. Delegation also needs a host with subagent and task backends. The default in-memory framework host does not supply these services; unregistered capability references remain inert.

See [Harnesses](/features/harnesses/) for selection and inheritance.
