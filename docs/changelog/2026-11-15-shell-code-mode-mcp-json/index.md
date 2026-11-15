---
title: "Shell Code Mode prints MCP replies as plain JSON"
description: "A tools call to an MCP server now prints the server's JSON reply itself, so scripts pipe it straight into jq instead of unwrapping a result string first."
date: 2026-11-15
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/shell-code-mode/, /framework/code-mode/]
---

In [Shell Code Mode](/capabilities/shell-code-mode/), a `tools` call to an MCP
server used to print a text reply wrapped as `{"result": "<text>"}`. Agents had
to unwrap it with `jq '.result | fromjson'` before they could use the data, and
often spent a turn finding that out. The call now prints the reply itself:

```bash
tools acme billing-list-invoices '{"status":"overdue"}' | jq '.items | length'
```

Direct MCP tool calls the model makes are unchanged.
