---
title: Give a Daytona or E2B sandbox a desktop
description: A Sandbox Template option starts the Session's Sandbox with a Linux desktop the agent sees and operates with the computer tool, next to its shell and files.
date: 2026-10-25
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/sandbox-templates/]
  prs: [everruns/everruns#4921, everruns/everruns#4923]
---

Daytona and E2B Sandbox Templates have a **Desktop** switch. With it on, the Session's Sandbox
starts with a Linux desktop and the agent gets the `computer` tool to take screenshots and click,
type and scroll on it. The desktop is the same machine as the shell and file tools and follows the
template's outbound network rule. The desktop is running as soon as the Sandbox is, and the shell
points at it, so a browser the agent starts from the shell opens on screen. Desktop computer use no longer needs a feature flag or a second
sandbox.

```json
{
  "target": {
    "kind": "managed",
    "provider": "e2b",
    "options": {
      "desktop": true
    }
  }
}
```

See [Sandbox Templates](/features/sandbox-templates/#desktop).
