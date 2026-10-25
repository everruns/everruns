---
title: MCP client authorization follows callback redirects
description: Approving an MCP client now completes even when its callback forwards the browser to another site, app, or local port after receiving a code.
date: 2026-10-18
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/getting-started/use-in-ai-tools/]
---

When you approve an MCP client such as Cursor, the browser can now follow the
client's callback through additional redirects. Previously the authorization
code could reach the registered callback while the browser stayed on the
Everruns consent page, making **Authorize client** appear to do nothing.

The consent page still validates the registered callback and binds approval to
your signed-in session. To connect or retry a client, follow
[Use Everruns in AI tools](/getting-started/use-in-ai-tools/).
