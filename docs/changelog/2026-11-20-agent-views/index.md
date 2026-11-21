---
title: "Published Agent views in Platform Chat and Work Chat"
description: "Configure an A2UI or HTML view on an Agent so scheduled runs can publish results in Platform Chat, beside Work Chat, or on a channel landing page."
date: 2026-11-20
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, agent]
links:
  docs: [/features/channels/]
---

Enable **Agent views** in organization Features, then open **More → Views** in
the Agent editor. Choose an A2UI or HTML layout, declare its data fields, and
preview it. Add the **Published views** capability so successful scheduled runs
can update the view.

For example, a scheduled housekeeping Agent can call `publish_view` with data
for the configured `checked` and `status` fields.

```json
{
  "data": {
    "checked": 4,
    "status": "Two changes need review"
  }
}
```

Open the view in Platform Chat or share it in a [Work Chat channel](/features/channels/)'s side panel
or landing page. If the author enables personalization, each signed-in viewer
can preview and save their own layout.

![A Work Chat landing page showing the findings from a scheduled repository sweep](./work-chat-published-view@2x.png)
