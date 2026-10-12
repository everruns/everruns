---
name: everruns-agent-authoring
description: Create or revise Everruns platform agents through MCP, including instructions, harnesses, capabilities, model selection, portable packages, and behavioral verification. Use when changing an agent definition; use everruns-mcp for running an existing agent or inspecting sessions.
license: MIT
compatibility: A connected Everruns MCP endpoint with discover, query, and execute tools and permission to manage agents.
metadata:
  category: platform
  version: "1.0"
  docs: https://everruns.com/docs/reference/cli/agents/
---

# Author an Everruns agent

Use the connected MCP server's `discover`, `query`, and `execute` tools.
Their `everruns` commands execute on the server; do not require a local CLI or
send REST requests with the MCP OAuth token. Select the user's organization and
pass `organization_id` consistently.

## Establish the behavior and dependencies

Translate the user's request into instructions, permitted actions, expected
outputs, and conditions that require human input. Resolve only missing details
that materially affect behavior. For an existing agent, read its current
definition and manager context before editing; preserve unrelated configuration.

Inspect available harnesses, capabilities, models, skills, and MCP catalog
entries through resources or discovered list/get commands. A harness supplies
the base environment; an agent overlays behavior and configuration. Reuse a
suitable harness and add capabilities only for required actions. Choose a model
available in the target organization and preserve the user's explicit choice.

MCP catalog entries are presets, not automatically active tools. Discover the
agent MCP attachment commands before adding a server. Select `actsAs` according
to whose account should perform the work: `user` requires the chatting person's
connection, `service` uses the agent's identity, and `user_or_service` explicitly
permits fallback. Unattended schedules require a usable service identity. Let
the user authorize connections through Everruns rather than collecting tokens.

## Apply a focused definition

Discover the exact deployed command and use its returned usage and input schema.
For a simple new agent, `everruns agents create` accepts `--name` and
`--system-prompt` in the server command tree; follow discovered usage if the
deployed version differs. For a portable definition, `everruns agents import --content` supports
inline content through MCP; discover its format and target options first.
Check for an existing name before creating or importing to avoid an unintended
upsert.

Portable packages keep one `agent.toml` definition, initial files, and complete
`.agents/skills/<name>/` trees. Supporting references and scripts belong with
their SKILL.md. Use the package import/export and validation operations offered
by the server rather than creating a parallel representation. Never embed
credentials in a definition or skill. Discover command support for validation,
diff, and preview; some file-oriented CLI commands require local files and
cannot be used directly by an MCP-only client.

For an update, carry the manager-context revision you read and a concrete
`--reason` describing the requested change. Read back the stored definition and,
where available, preview its effective harness and capability composition.
Configuration saved successfully is not evidence that the behavior works.

## Verify the behavior

When authorized to run the agent, use `agent_run` with a representative request,
then inspect the session output and events. Verify the requested result, the
tools used, and whether a connection, secret, question, or approval blocked
progress. Use an available analyze/health-check operation when its cost and side
effects fit the task; discover its contract first. Do not claim a live test
when you only validated or previewed the definition.

Test side-effecting behavior within the user's authorized scope. A request to
create a scheduled agent does not imply permission to perform its external
action immediately. Configure schedule/channel publication only when requested,
using discovered commands, and report whether it is saved, verified, connected,
and active.

If interrupted after a write, query the stored state before retrying. Summarize
the final behavior and unresolved requirements, with the returned agent or
session link when available.
