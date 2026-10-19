---
name: everruns
description: Understand Everruns products, core concepts, and where to find documentation. Use for product questions, getting started, or choosing between Cloud, the self-hosted Platform, and the Rust Framework.
license: MIT
metadata:
  category: platform
  version: "4.3"
  homepage: https://everruns.com
  docs: https://everruns.com/docs/
  repository: https://github.com/everruns/everruns
---

# Everruns

Everruns is an open-source agent framework, runtime, and control plane. Use
[Everruns Cloud](https://app.everruns.com) for the managed service, the
[self-hosted Platform](https://everruns.com/docs/getting-started/docker-compose/) to operate
your own deployment, or the [Rust Framework](https://everruns.com/docs/framework/)
to run agents inside an application.

## Core concepts

- **Agent**: a reusable definition of behavior, instructions, model selection,
  capabilities, and an execution harness.
- **Harness**: a reusable execution setup. Conversation supports dialogue;
  Worker adds tools and files; Bashkit Worker adds a virtual shell; Sandbox
  Worker supplies a full sandbox. Reuse an appropriate built-in harness.
- **Capability**: tools, context, files, or behavior added to an agent or
  harness. Availability does not prove that an agent has it attached or that
  the person has connected the required account.
- **Session**: a conversation or run with its own messages, events, state, and
  workspace. Creating a session is distinct from completing its task.
- **Workspace**: durable files used by an agent. A virtual workspace does not
  imply native processes, packages, or network services are available.
- **Trigger**: a schedule or webhook that starts work on an agent. Use Agent
  Triggers for recurring autonomous work.
- **Organization**: the boundary for shared platform resources and permissions.
  Personal connections and private memory retain their own access rules.

## Find authoritative guidance

Start with the [documentation](https://everruns.com/docs/) for product behavior
and configuration. For an agent-readable index, use
[llms.txt](https://everruns.com/docs/llms.txt), then read the relevant topic or
page. Consult documentation for the version in use; current organization state
comes from the connected deployment's list/get operations.

Useful starting points:

- [Getting started](https://everruns.com/docs/getting-started/introduction/)
  and [core concepts](https://everruns.com/docs/getting-started/concepts/)
- [Harnesses](https://everruns.com/docs/features/harnesses/)
- [Capabilities](https://everruns.com/docs/capabilities/)
- [CLI usage](https://everruns.com/docs/features/cli/) and
  [command reference](https://everruns.com/docs/reference/cli/)
- [MCP](https://everruns.com/docs/features/mcp/) and
  [Cloud authentication guidance](https://everruns.com/auth.md)
- [Agent packages](https://everruns.com/docs/how-to/define-agents-as-files/)
- [Agent Triggers](https://everruns.com/docs/features/agent-triggers/)
- [Rust Framework](https://everruns.com/docs/framework/)

## Choose a workflow skill

Use the host's skill loader to activate the guide that matches the task:

| Skill | When to load it |
|---|---|
| `everruns-cli` | Use the installed CLI or an agent's command shell; discover commands, script operations, inspect resources, and verify changes. Explains how CLI, session-shell, and MCP invocation differ. |
| `everruns-mcp` | Connect through an external Everruns MCP server; select an organization, run and follow sessions, handle questions and connections, and use MCP resources. Refers to `everruns-cli` for command usage. |
| `everruns-agent-authoring` | Create or revise an agent's behavior, choose its harness, capabilities, model, and identity, work with portable packages, and verify the result. |
| `everruns-framework` | Develop Rust applications that run agents in process with the `everruns` crate, or compose advanced execution hosts with `everruns_core::host`. Includes crate-selection and domain-language references. |

Activate only skills offered by the current host and relevant to the task.
This guide provides orientation; companion skills supply the procedures for
working through a CLI, MCP connection, application SDK, or Rust runtime.

### Additional coding-agent plugin workflows

The [Everruns plugin](https://github.com/everruns/plugins) also provides focused
task guides. Activate them when installed and relevant:

| Skill | When to load it |
|---|---|
| `everruns-run-agent` | Hand a self-contained task to an existing agent, follow progress, exchange files, and bring back the result. |
| `everruns-build-agent` | Build or improve an agent from a plain-language brief, select tools and knowledge, and smoke-test it. Complements the shared authoring guide. |
| `everruns-debug-session` | Diagnose a failed, stalled, or poor-quality session and verify the fix. |
| `everruns-evaluate-agent` | Build regression evals from real sessions, compare models, and score live traffic. |
| `everruns-ship-agent` | Expose an agent through channels and triggers, with budgets and controls for stopping it. |
| `everruns-operate` | Investigate organization spending, health, and change history. |
| `everruns-agents-as-files` | Keep agent packages in a repository and validate, compare, import, or export them. |
| `everruns-sdk` | Call hosted Everruns agents from Python, TypeScript, or Rust application code. Use `everruns-framework` when the Rust application runs the agent itself. |
