---
name: agent-card
description: Render an Everruns(Dev) MCP-Apps card for an agent (sandboxed HTML stats card)
argument-hint: "<agent_id_or_name>"
---

Call the `agent_get_card` MCP tool with the supplied positional argument as
`agent_id`. The tool accepts either a prefixed agent ID
(`agent_{32-hex}`) or a unique agent name within the default organization, or
within the target organization when the user provides an `organization_id`.

The result is an MCP `resource` content block at
`ui://everruns/agent/{agent_id}/card` (`text/html`), plus a one-line text
summary. In MCP-UI-aware hosts the resource renders as a sandboxed iframe
with the agent's name, description, status, tags, token usage and session
count. In hosts that ignore embedded resources, surface the summary line.

If the agent reference cannot be resolved, fall back to `query` with
`list_agents` to find the right ID, then retry. See the `everruns-dev`
skill for details and `knowledge/ui/mcp-cards.md` for the card standard.

Arguments: `$ARGUMENTS`
