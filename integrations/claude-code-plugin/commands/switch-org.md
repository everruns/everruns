---
description: Switch the active Everruns organization for this Claude Code session
argument-hint: "<organization_id>"
---

Call the `switch_organization` MCP tool with the provided `organization_id` (format `org_{32-hex}`).

Arguments: `$ARGUMENTS`

- If no id is provided, call `list_organizations` and ask the user to pick.
- `switch_organization` is advisory — the MCP transport is stateless. Remind the user to pass `--organization_id <id>` to subsequent commands (or supply it in the raw tool calls) to stay in that org.
- After a successful switch, echo the validated org name, slug and id.
