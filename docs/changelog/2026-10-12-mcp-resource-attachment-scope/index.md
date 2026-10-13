---
title: "MCP resource reads enforce conversation attachments"
description: "Resource listing and reading retain the conversation's attached-server policy when the runtime binds a request to its current input message."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, framework, cloud]
surfaces: [agent]
links:
  docs: [/features/mcp/#reading-a-servers-resources]
---

MCP resource listing and reading now retain the conversation's attached-server
policy through tool registration and execution. The same policy applies when
the runtime binds a request to the input message that started the turn.

Allowed resources continue to use the server's configured sign-in and network
rules. Deferred servers remain available when attached. Attach a server to the
conversation before asking the agent to read its resources; see
[Reading a server's resources](/features/mcp/#reading-a-servers-resources).
