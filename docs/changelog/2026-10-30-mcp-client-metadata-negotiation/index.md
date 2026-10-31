---
title: MCP OAuth clients can use hosted metadata documents
description: Outbound MCP OAuth clients can select a hosted client metadata document when supported, while preserving configured clients and dynamic registration fallback.
date: 2026-10-30
kind: feature
significance: minor
products: [framework, yolop]
surfaces: [code, cli]
links:
  docs: [/framework/host-services/]
---

Outbound MCP OAuth login can identify a client with a hosted HTTPS metadata
document instead of creating a registration at each authorization server.
Framework hosts supply the document URL through `prepare_login_with_options`.
Yolop distributors can configure it before login:

```bash
YOLOP_MCP_CLIENT_METADATA_URL=https://client.example/yolop.json yolop mcp login my-server
```

Publish the document first, with its exact URL as `client_id`, a client name,
and the allowed callback URIs. Configured clients and saved registrations keep
precedence. Servers that do not advertise metadata-document support continue
through dynamic registration. PKCE, callback validation, and token refresh keep
the same behavior. See [Framework host services](/framework/host-services/).
