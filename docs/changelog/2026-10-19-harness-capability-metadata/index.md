---
title: Harness capabilities keep their name and details
description: A harness row for a capability hidden by a rollout flag now shows its name, icon, and description, and links to the capability page.
date: 2026-10-19
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs:
    - /capabilities/agent-skills/
    - /features/harnesses/
---

A harness such as Worker can include [Agent Skills](/capabilities/agent-skills/) while that capability's rollout flag is off. The catalog then omitted it, so the harness page showed the bare id `skills` with no name, icon, or description.

Management lists and the capability page now return that metadata anyway. Each row on a [harness](/features/harnesses/) links the name and More details to the capability page. The discovery catalog still hides a capability whose flag is off, and saving a harness still refuses to turn one on.

```json
{
  "id": "skills",
  "name": "Agent Skills",
  "description": "Discover and activate skills from the session filesystem.",
  "icon": "wand",
  "feature_flag": "skills"
}
```
