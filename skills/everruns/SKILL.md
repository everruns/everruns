---
name: everruns
description: Everruns, the open-source runtime for durable AI agents. Build agents into Rust applications with the everruns crate; install, upgrade, or remove a self-hosted Everruns with Docker or Kubernetes; and create, run, debug, evaluate, and ship agents on it through the everruns CLI, the MCP server, the REST API, or the SDKs. Use for any Everruns task, from product questions and getting started to agent definitions, sessions, channels, triggers, costs, and health.
license: MIT
compatibility: Framework work needs Rust 1.96+. Platform work needs the everruns CLI, an Everruns MCP connection, or HTTP access to an Everruns deployment.
metadata:
  category: platform
  version: "6.3"
  homepage: https://everruns.com
  docs: https://everruns.com/docs/
  repository: https://github.com/everruns/everruns
---

# Everruns

Everruns is a free, MIT-licensed community project: an agent framework, a
durable runtime, and a control plane. People run it themselves, in one of two
ways:

- **Framework**: the `everruns` Rust crate runs agents inside the person's own
  application. No server, database, or worker.
- **Platform**: the shared runtime, workers, UI, and API on the person's
  machine (`everruns docker up`), a server, or Kubernetes (`everruns k8s up`).
  People and other programs reach it through the UI, the `everruns` CLI, MCP,
  the REST API, and SDKs.

A demo instance at <https://app.everruns.com> runs the same code so people can
look around in a browser. It is a demo, not a hosted service: use it only when
the person asks for it, and never as a fallback for their own install.

## Read the guide for the task

This file is the map. Each task has a guide under `references/`; read the ones
the task needs before acting, and nothing else.

| The person wants to | Read |
|---|---|
| Build agents into a Rust application | [framework.md](references/framework.md), then [framework-crates.md](references/framework-crates.md) and [framework-glossary.md](references/framework-glossary.md) for advanced hosts |
| Install, upgrade, check, or remove their own Everruns | [install.md](references/install.md) |
| Use the `everruns` CLI or a session shell; script operations | [cli.md](references/cli.md) |
| Connect a coding agent or MCP client; work through MCP tools | [mcp.md](references/mcp.md) |
| Call a deployment over HTTP with a token | [api.md](references/api.md) |
| Call Platform agents from Python, TypeScript, or Rust code | [sdk.md](references/sdk.md) |
| Build or improve an agent from a plain-language brief | [build-agent.md](references/build-agent.md) |
| Design or revise an agent definition: harness, capabilities, model, packages | [agent-authoring.md](references/agent-authoring.md) |
| Let an agent start other agents: one-off calls, followed threads, a coordinator | [agents-starting-agents.md](references/agents-starting-agents.md) |
| Hand a task to an existing agent and bring back the result | [run-agent.md](references/run-agent.md) |
| Find out why a session failed, stalled, or answered badly | [debug-session.md](references/debug-session.md) |
| Choose Bashkit, Monty Python Code Mode, native tools or session sandboxes | [execution-options.md](references/execution-options.md) |
| Build regression evals, compare models, score live traffic | [evaluate-agent.md](references/evaluate-agent.md) |
| Put an agent on Slack, the web, A2A, webhooks, or a schedule | [ship-agent.md](references/ship-agent.md) |
| Check spending, health, or who changed what | [operate.md](references/operate.md) |
| Keep agent definitions as files in a repository | [agents-as-files.md](references/agents-as-files.md) |

If the person only says they want to start with Everruns, ask whether they are
adding agents to a Rust application (Framework) or want a running Platform
(install). If they want to build an agent but have no deployment, offer the
install first; offer the demo only as a way to look around.

## Rules for every task

- **The person's own Everruns first.** Use the deployment the person names, or
  the Everruns MCP server already connected in this session, or the one the
  CLI is signed in to (`everruns status`), or `http://localhost:9300` when
  `curl -fsS http://localhost:9300/health` answers. Ask only when none of these
  applies; a connected server is the person's choice, so work with it.
- **Trust the installed version, not memory.** Everruns changes quickly. Check
  `everruns --version` or the server's `/health`, read `everruns <noun> --help`
  or MCP `discover` before using a command, and read docs.rs for the crate
  version Cargo resolved. Guides here show the shape of a task, not every flag.
- **Ask before changing anything outside the conversation**: installs, agents,
  channels, schedules, budgets. Read-only checks need no approval.
- **Keep secrets out of the conversation.** API keys and tokens go in the
  person's shell or an Everruns setup page, never in chat, files, or commands
  you echo back.
- **Every change carries a reason.** Platform changes are recorded in the
  entity's history: read the manager context first (`everruns context get
  <id>`), then pass `--reason` and `--context-revision`.
- **Look at the examples before building from scratch.** Ready-made agents
  and harnesses ship as examples that an organization adopts, so they are not
  in `agents list` or `harnesses list` until someone does. Coding, the
  Yolop-based coding harness, is one of them. List them with `everruns agents
  examples list` and `everruns harnesses examples list` (MCP operations
  `list_agent_examples` and `list_harness_examples`), adopt the closest with
  `examples adopt <name>`, and adapt it.
- **End with a check the person can see**: a health check, a session result, a
  link. Saved configuration is not proof that an agent works.

## Core concepts

- **Agent**: a reusable definition of behavior, instructions, model selection,
  capabilities, and an execution harness.
- **Example**: a ready-made agent or harness shipped with the deployment.
  Adopting one creates an ordinary, editable agent or harness in the
  organization; until then it only appears in the examples list.
- **Harness**: a reusable execution setup. Conversation supports dialogue;
  Worker adds tools and files; Bashkit Worker adds a virtual shell; Sandbox
  Worker supplies a full sandbox. Reuse an appropriate built-in harness, or
  adopt a harness example (Coding for software work, Data Analyst for data)
  when one fits: `everruns harnesses examples list`.
- **Capability**: tools, context, files, or behavior added to an agent or
  harness. Availability does not prove that an agent has it attached or that
  the person has connected the required account.
- **Spawn agents and Threads**: the two ways an agent starts other agents,
  never both on one agent. Spawn agents makes one-off calls (brief in, result
  out); Threads hands off followed work the agent messages and closes.
- **Session**: a conversation or run with its own messages, events, state, and
  workspace. Creating a session is distinct from completing its task.
- **Workspace**: durable files used by an agent. A virtual workspace does not
  imply native processes, packages, or network services are available.
- **Trigger**: a schedule or webhook that starts work on an agent. Use Agent
  Triggers for recurring autonomous work.
- **Organization**: the boundary for shared platform resources and permissions.
  Personal connections and private memory retain their own access rules.

## Documentation

- [Documentation](https://everruns.com/docs/), and its agent-readable index
  [llms.txt](https://everruns.com/docs/llms.txt). Request any page with
  `Accept: text/markdown` to get Markdown.
- [Start here](https://everruns.com/docs/getting-started/introduction/) and
  [core concepts](https://everruns.com/docs/getting-started/concepts/)
- [Framework](https://everruns.com/docs/framework/) and the
  [crate API](https://docs.rs/everruns)
- [CLI reference](https://everruns.com/docs/reference/cli/),
  [MCP](https://everruns.com/docs/features/mcp/), and
  [REST API](https://everruns.com/docs/api/)
- A deployment's own `/llms.txt` and `/auth.md` describe that deployment.

This skill is published in [everruns/everruns](https://github.com/everruns/everruns/tree/main/skills/everruns),
installable with `npx skills add everruns/everruns`, bundled in the
[Everruns plugin](https://github.com/everruns/plugins), served by every
Everruns MCP server as `skill://everruns/SKILL.md`, and written into a project
by `everruns agent-skill install`.
