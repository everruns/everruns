---
title: Clearer screens for adding and managing agent channels
description: New channel groups types by who uses them, the Integrations tab lists channels as rows, and each channel gets its own page to connect and change settings.
date: 2026-10-29
kind: improvement
significance: minor
products: [platform, cloud]
release: "0.50.0"
surfaces: [ui]
links:
  docs: [/features/channels/]
---

**Add channel** now asks who will use the channel. Types are grouped under
**For people** and **For software**, each with one plain sentence and what it
needs first, such as a Slack workspace. The form after it is split into named
sections, and rarely changed limits fold behind a summary.

On the agent's **Integrations** tab, channels are rows showing status and when
each was last used. A row opens the channel's page: **Connect** shows the link,
request URL or code to use it, and **Settings** edits it with Save changes and
Discard. See [Channels](/features/channels/).

![The New channel page with channel types grouped for people and for software](./new-channel@2x.png)
