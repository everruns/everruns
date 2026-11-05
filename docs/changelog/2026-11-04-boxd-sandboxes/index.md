---
title: Run Sandboxes on boxd machines that keep working while idle
description: boxd is a Sandbox Template target and an agent capability, with region and size choices and hibernation that keeps running processes.
date: 2026-11-04
release: "0.51.0"
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api, agent]
links:
  docs: [/integrations/boxd/, /features/sandbox-templates/]
---

[boxd](https://boxd.sh/) machines are now a managed target in
[Sandbox Templates](/features/sandbox-templates/).
Connect a boxd API key, pick **boxd** under **Runs in**, and choose a region
(Frankfurt or New York) and a size. Each session gets a full Linux VM with
Docker. When the session goes idle, Everruns hibernates the machine, which keeps
memory and running processes and bills only for disk until the next command
wakes it. Template network rules become the machine's egress allowlist.

```json
{
  "target": {
    "kind": "managed",
    "provider": "boxd",
    "options": {
      "region": "nyc",
      "size": "4x16"
    }
  },
  "durability": "provider_snapshot"
}
```

Agents can also manage machines themselves with the `boxd` capability. See
[boxd](/integrations/boxd/).
