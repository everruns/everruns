---
title: "Set an agent's avatar from the CLI and MCP"
description: "The set_agent_avatar command takes a curated preset id or a base64 image, so an agent building another agent over MCP can give it a picture."
date: 2026-10-12
kind: feature
significance: minor
products: [platform, cloud, cli]
surfaces: [cli, agent, api]
release: "0.48.0"
links:
  docs: [/reference/cli/agents/]
---

Agent avatars could only be set from the UI or the multipart upload route. Two
commands now cover the CLI and MCP (see [`everruns agents`](/reference/cli/agents/)): `list_avatar_presets` lists the curated
catalog, and `set_agent_avatar` sets either a preset or your own PNG, JPEG, GIF
or WebP image, sent as base64 and at most 1 MB decoded. The agent's one-click
Slack apps get the new icon, as with an upload.

```bash
everruns agents avatar presets
everruns agents avatar set agent_01h9 --preset familiars-patch --reason 'Brand the triage agent'
base64 logo.png > logo.b64
everruns agents avatar set agent_01h9 --image-base64 @logo.b64 --reason 'Use the team logo'
```

Over REST the same command is `POST /v1/agents/{agent_id}/avatar/set`.
