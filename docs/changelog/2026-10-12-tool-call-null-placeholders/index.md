---
title: "Tool-call repair drops null placeholders on optional parameters"
description: "When a model fills optional tool parameters with null or the string null, tool-call repair now leaves them out instead of letting a strict tool reject the call."
date: 2026-10-12
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [agent]
links:
  docs: [/capabilities/tool-call-repair/]
  prs: []
---

Some models write `null` or `"null"` for every optional parameter they mean to
skip. Strict tools, such as MCP servers that validate arguments against their
schema, rejected those calls and the model had to retry. With the Tool Call
Repair capability on, an optional parameter whose schema does not accept null
is now left out of the call, so the first attempt goes through. Required
parameters, and parameters that accept null, are passed as written.

See [Tool Call Repair](/capabilities/tool-call-repair/).
