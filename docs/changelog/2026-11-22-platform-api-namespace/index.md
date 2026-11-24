---
title: Platform REST APIs use a dedicated namespace
description: Update platform REST calls to /api/v1/platform. Console, CLI and SDK platform clients use the new paths; published agent endpoints keep their existing URLs.
date: 2026-11-22
kind: deprecation
significance: notable
products: [platform, cloud, cli, sdk]
surfaces: [api, ui]
links:
  docs: [/features/cli/, /features/sdk/]
---

Platform REST endpoints now live under `/api/v1/platform/`. For example,
`/api/v1/agents` becomes `/api/v1/platform/agents`, and operator access to
sessions uses `/api/v1/platform/sessions`. Update direct HTTP integrations
to the canonical namespace. The updated [CLI](/features/cli/) and
[SDK platform clients](/features/sdk/) already use it.

Legacy management URLs continue to work with the same handlers, permissions
and rate limits. They become deprecated on **December 22, 2026**. Responses
include a `Deprecation` header announcing that date and `Link` headers pointing
to the canonical URL and this guide. The date does not automatically remove
the legacy URLs; a removal date will be announced separately. Existing CLI
and SDK releases continue working during the transition.

```http
# Supported legacy alias
GET /api/v1/agents

# Canonical
GET /api/v1/platform/agents
```

The console, CLI and Python, TypeScript and Rust SDK platform clients use the
new namespace. Resource permissions and personal access token scopes are
unchanged. Published Agent API, AG-UI and A2A channel URLs remain unchanged,
as do inbound webhooks, registered OAuth callbacks and runtime self-service.
