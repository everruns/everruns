---
title: "Everruns SDKs drop the agent version methods"
description: "The agent version endpoints are gone from the API, so the Rust, Python and TypeScript SDKs remove their version methods. Use change history instead."
date: 2026-10-13
kind: breaking
significance: notable
products: [sdk]
surfaces: [code]
links:
  docs: ["/features/change-history/"]
  prs: [everruns/sdk#123]
---

The server replaced the `/v1/agents/{id}/versions` endpoints with the generic
[change history](/features/change-history/), so the SDKs no longer offer
`list_versions`, `create_version`, `set_default_version`, `diff_versions`,
`fork_version` or `rollback_version` (camelCase in TypeScript). Code that
called them failed against current servers anyway.

Before:

```text
await client.agents.list_versions("agent_123")
```

After, read the agent's history over REST until the SDKs wrap it:

```bash
curl -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  https://app.everruns.com/api/v1/history/agent_123
```

`MessageInput` now only takes the `user` role; send tool results with
`create_tool_results`.
