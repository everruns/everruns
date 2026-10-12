---
title: "Agents get avatars from presets or an uploaded image"
description: "Give an agent an avatar from 25 curated presets or a cropped upload, and it shows in the console, on the A2A Agent Card and as the Slack app icon."
date: 2026-10-05
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.42.0"
links:
  docs: [/features/ui/]
  prs: [everruns/everruns#4198, everruns/everruns#4220]
visual: "Agent page Branding sheet with the Choose preset avatar grid open (/agents/<id>), cropped to the sheet"
---

**In the UI:** open an agent's **Branding** sheet. Drop a PNG, JPEG, GIF or WebP
image on the avatar and position the square crop, or use **Choose preset** to
search 25 curated avatars and save one with **Use avatar**.

**In code:** `PUT /v1/agents/{agent_id}/avatar` uploads an image as the
multipart field `file`, and `PUT /v1/agents/{agent_id}/avatar/preset` selects a
preset. The agent's `avatar` field lists public, immutable image URLs. The A2A
Agent Card carries it as `iconUrl`, and one-click Slack apps use it as their icon.

See [Web UI](/features/ui/) and [#4198](https://github.com/everruns/everruns/pull/4198), [#4220](https://github.com/everruns/everruns/pull/4220).
