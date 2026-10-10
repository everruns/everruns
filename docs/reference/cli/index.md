---
title: CLI command reference
description: Every everruns CLI command, grouped by what it manages, with flags and a worked example.
sidebar:
  label: Overview
  order: 0
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Every command below is also in `everruns --help`, and an agent running on Everruns types the same words in its shell. Install the CLI and sign in first; see [CLI](/features/cli/).

## Flags every command takes

| Flag | Description |
|---|---|
| `--reason <TEXT>` | Why you are making this change. Recorded in the changed entity's history; give one on every command that changes something. |
| `--context-revision <N>` | The revision of the entity's manager notes you read (`everruns context get`). The change is refused if the notes changed since. |
| `-o`, `--output <FORMAT>` | `text` (default), `json` or `yaml`. |
| `-q`, `--quiet` | Print only the essential identifier, for capturing ids in scripts. |

## Session and account

| Command | What it does |
|---|---|
| `everruns chat` | Send a message and stream the response |
| `everruns login` | Interactive login (localhost OAuth callback) |
| `everruns logout` | Remove stored credentials |
| `everruns status` | Show current user and org |

## Command groups

| Group | What it manages |
|---|---|
| [`agents`](/reference/cli/agents/) | Agent definitions and their configuration |
| [`budgets`](/reference/cli/budgets/) | Spending limits for sessions, agents, users and organizations. |
| [`capabilities`](/reference/cli/capabilities/) | Capabilities agents can use, including declarative ones you define. |
| [`connections`](/reference/cli/connections/) | Your LLM provider API keys |
| [`context`](/reference/cli/context/) | Notes managers keep about an entity, read by agents that manage it. |
| [`durable`](/reference/cli/durable/) | Durable scheduled tasks and their executions. |
| [`evals`](/reference/cli/evals/) | Evaluation cases, runs, results and scores. |
| [`files`](/reference/cli/files/) | Sync files between a local folder and a session |
| [`harnesses`](/reference/cli/harnesses/) | Reusable base setups (prompt plus capabilities) agents build on. |
| [`health-issues`](/reference/cli/health-issues/) | Operational problems detected in this installation. |
| [`history`](/reference/cli/history/) | Recorded changes to entities: list, compare, restore. |
| [`images`](/reference/cli/images/) | Uploaded images. |
| [`knowledge-bases`](/reference/cli/knowledge-bases/) | Curated collections of knowledge entries. |
| [`knowledge-indexes`](/reference/cli/knowledge-indexes/) | Searchable indexes synced from external sources. |
| [`mcp-servers`](/reference/cli/mcp-servers/) | Registered MCP servers available to agents and sessions. |
| [`memories`](/reference/cli/memories/) | Workspace memories in the organization. |
| [`models`](/reference/cli/models/) | LLM models available to agents, and organization defaults. |
| [`notifications`](/reference/cli/notifications/) | Your in-app notifications. |
| [`observers`](/reference/cli/observers/) | Online scoring of live traces. |
| [`orgs`](/reference/cli/orgs/) | Organizations you belong to, and which one is active |
| [`payments`](/reference/cli/payments/) | Machine payments: wallets, spend policies and attempts. |
| [`plugin-marketplaces`](/reference/cli/plugin-marketplaces/) | Sources of installable plugins. |
| [`plugins`](/reference/cli/plugins/) | Plugins installed in this organization. |
| [`providers`](/reference/cli/providers/) | LLM providers and their credentials. |
| [`reports`](/reference/cli/reports/) | Usage and cost reporting: queries, saved reports, catalog. |
| [`sandbox-targets`](/reference/cli/sandbox-targets/) | Sandbox providers this deployment offers. |
| [`sandboxes`](/reference/cli/sandboxes/) | Sandboxes across providers: state, history, usage. |
| [`sessions`](/reference/cli/sessions/) | Running and archived sessions, their state and participants |
| [`skills`](/reference/cli/skills/) | Skill packages and their content. |
| [`system`](/reference/cli/system/) | Server status. |
| [`tasks`](/reference/cli/tasks/) | Background tasks across every session in the organization. |
| [`user`](/reference/cli/user/) | Your own account settings. |
| [`users`](/reference/cli/users/) | People in the current organization. |
| [`virtual-users`](/reference/cli/virtual-users/) | Runtime accounts for consumers or services. |
| [`workspaces`](/reference/cli/workspaces/) | Durable working areas holding the files agents work on. |
