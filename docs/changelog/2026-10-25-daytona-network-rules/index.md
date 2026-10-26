---
title: Limit outbound network for Daytona and E2B sandboxes
description: Daytona and E2B Sandbox Templates can block the network or allow only listed domains, enforced by the provider together with the Session's network access list.
date: 2026-10-25
kind: feature
significance: minor
products: [platform, cloud]
release: "0.49.0"
surfaces: [ui, api]
links:
  docs: [/features/sandbox-templates/]
  prs: [everruns/everruns#4913, everruns/everruns#4914]
---

Daytona and E2B Sandbox Templates now offer **Outbound network**: open, only listed domains, or
blocked. The provider enforces the rule on its own network, so it covers every process in the
Sandbox, not only Everruns tools. The Session's network access list narrows it further, and a
resumed Sandbox follows the current list. Sandboxes the agent creates with Daytona or E2B tools,
including the desktops, get the Session's list too.

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
