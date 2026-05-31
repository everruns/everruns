---
name: whoami
description: Show the current Everruns user and default organization
---

Call the `me` MCP tool. If it fails with an auth error, tell the user to
complete the OAuth flow their host opens in the browser. For multi-org users,
explain that MCP has no current-org switch; use `list_organizations` and pass
`organization_id` on each org-scoped tool call to target a non-default org.
