---
title: Limit outbound network for Daytona sandboxes
description: Daytona Sandbox Templates can block the network or allow only listed domains, and Daytona enforces it together with the Session's network access list.
date: 2026-10-25
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/sandbox-templates/]
  prs: [everruns/everruns#4913]
---

Daytona Sandbox Templates now offer **Outbound network**: open, only listed domains, or blocked.
Daytona enforces the rule on its own network, so it covers every process in the Sandbox, not
only Everruns tools. The Session's network access list narrows it further, and a resumed Sandbox
follows the current list. Sandboxes the agent creates with Daytona tools, including the desktop,
get the Session's list too.

```json
{
  "containment": {
    "level": "isolated",
    "network": {
      "mode": "allowlist",
      "allowed_hosts": [
        "github.com",
        "*.github.com"
      ]
    }
  }
}
```

See [Sandbox Templates](/features/sandbox-templates/#outbound-network).
