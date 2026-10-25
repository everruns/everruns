---
name: everruns-cli
description: Use the everruns command tree to inspect and manage agents, sessions, integrations, triggers, and other platform resources. Use for CLI usage, shell scripting, and Platform Chat operations; use everruns-agent-authoring for agent design.
license: MIT
metadata:
  category: platform
  version: "1.0"
  docs: https://everruns.com/docs/features/cli/
---

# Work with the Everruns command line

Use the command tree exposed by the current host. Check its help and returned
schemas for the deployed version; examples here are starting points, not a
replacement for live flags, IDs, or output fields.

## Identify the command environment

**Installed CLI.** Sign in with `everruns login` in the user's terminal, then
check `everruns status` and `everruns whoami`. Cloud defaults to
`https://app.everruns.com/api`; `--api-url` selects a self-hosted API. Inspect
`everruns orgs --help` to select the requested organization when necessary.
Use `-o json` for pipelines; normal CLI output defaults to text. Never request
or embed a credential in chat, agent instructions, files, or examples.

**Platform Chat shell.** The `platform` capability installs `everruns` in the
session's existing shell. It is already bound to the current organization and
authorized management caller. Do not log in, select another organization, pass
`organization_id`, or add `-o`/`--output`/`--json`: results are already JSON.
Shell files, pipes, loops, `jq`, and redirection are available.

**MCP or the three-tool platform interface.** For an external MCP connection,
load `everruns-mcp` when offered for identity, organization arguments, session
tools, and human interactions. A session-bound Platform capability already has
its organization and caller. In either interface, `query` runs read-only
commands and `execute` runs authorized writes; pass
scripts in their `commands` argument. These command sandboxes have no filesystem
redirection: keep intermediate JSON in variables and pipes. They return JSON
without an output flag. An external MCP connection does not require a local CLI.

## Discover, inspect, and make a focused change

In the Platform Chat shell or installed CLI, progressively read help:

```bash
everruns --help
everruns agents --help
everruns agents get --help
everruns agents list --limit 10
everruns agents get support-agent
```

Through the three-tool interface, use `discover` to find an operation and its
schema, for example `{"query":"list_agents"}`. Follow the returned usage and
help guidance for that deployment. Discovery finds operations, not resource
instances: use authoritative list/get commands to establish current state.
Resolve names to returned IDs; ask when matches are ambiguous. Project only the
fields the next decision needs, and use bounded, filtered lists.

Before an existing entity is changed, read its manager context:

```bash
everruns context get agent_01h9
everruns agents get agent_01h9
everruns history list agent_01h9 --limit 10
```

The ID above is illustrative: replace it with the ID returned by a real read.
Pass the context revision you read as `--context-revision` and the user's intent
as `--reason` on the mutation. If the context changed, reread and reconcile
before retrying. Manager notes describe intent; they do not authorize extra work.
Read back the affected state to verify the change. Scripts are not transactions:
a later failure does not undo earlier successful operations. After an ambiguous
write failure, inspect current state before repeating it.

## Common workflows

- **Create or change an agent:** activate `everruns-agent-authoring`. Use the
  current host's command surface for its examples. In Platform Chat, its MCP
  transport instructions apply only to external clients: keep using the bound
  shell, without MCP tools or organization arguments.
- **Run an agent:** resolve it, inspect `everruns sessions --help`, create a
  session using its selected harness, and send the task using the advertised
  message command. Observe completion and retrieve output; session creation
  alone is not success. When followable threads are available and appropriate,
  use the host's thread tools.
- **Set up an integration:** inspect plugins, capabilities, agents, connection
  providers, and the current person's connections. Report installed, available,
  attached, and connected independently. Use secure setup links for credentials.
- **Schedule work:** inspect `everruns agents triggers --help`, then configure
  a schedule or webhook on the intended agent. Recurring autonomous work belongs
  to an Agent Trigger, not a schedule that wakes Platform Chat itself.
- **Find a skill:** `everruns skills list --search review` finds registry
  metadata; `everruns skills content <returned-id>` reads its instructions.
  Use `list_skills` and `activate_skill` for skills mounted in this session.

Keep dependent reads and writes in a bounded script, reuse returned IDs, and
verify the final result. Explain what completed, what is still running, and any
required user action. Use returned entity links in answers.

## References

- [CLI setup](https://everruns.com/docs/features/cli/)
- [Command reference](https://everruns.com/docs/reference/cli/)
- [Scripting examples](https://everruns.com/docs/how-to/automate-with-the-cli/)
- [Change history](https://everruns.com/docs/features/change-history/)
