---
title: "Label MCP tools as read only or changes things"
description: "Save a risk label on each tool of an organization MCP server to decide whether it asks for approval, with model-suggested labels a person can accept."
date: 2026-10-10
kind: feature
significance: notable
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
links:
  docs: [/features/mcp/#which-tools-ask-before-they-run, /reference/cli/mcp-servers/]
  prs: [everruns/everruns#4475, everruns/everruns#4498]
visual: "MCP catalog Tools dialog with Default, Read only and Changes things choices and a suggestion (/settings/mcp-catalog), cropped to the dialog"
---

Everruns cannot see inside a remote tool, so most MCP tools ask for approval by
default. A saved label now decides it: **Read only** never asks, **Changes
things** always asks. A label wins over the server's own hints and survives
tool list changes.

**In the UI:** open **Settings > MCP catalog** and click **Tools** on a server.
**Suggest labels** asks a decision model to judge unlabeled tools; nothing
changes until someone picks a choice or clicks **Use suggestion**.

**In code:** `PUT /v1/mcp-servers/{id}/tools/{tool_name}/label` with
`{"label": "read_only"}`, or `everruns mcp-servers label-tool` and
`suggest-labels` on the CLI. Label changes are in the server's history.

See [Which tools ask before they run](/features/mcp/#which-tools-ask-before-they-run)
and [#4475](https://github.com/everruns/everruns/pull/4475), [#4498](https://github.com/everruns/everruns/pull/4498).
