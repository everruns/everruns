---
title: "Generate an agent's avatar in one command"
description: "everruns agents avatar generate draws an avatar with the organization's default image model and sets it, from the CLI, MCP or API, without running a session."
date: 2026-11-20
kind: feature
significance: minor
products: [platform, cloud, cli]
surfaces: [cli, agent, api]
links:
  docs: [/reference/cli/agents/]
---

Getting a drawn avatar used to take two steps: run an agent session with image
generation, then pass the picture back through `avatar set --image-base64`.
[`everruns agents avatar generate`](/reference/cli/agents/#agents-avatar-generate)
(`generate_agent_avatar`) does both. It draws with the organization's default
image model (or GPT Image 2.6 Flare, then Muse Image, on the org's own
accounts, as the image tools do) and sets the result, so the agent's one-click
Slack apps get the new icon too.

```bash
everruns agents avatar generate agent_01h9 \
  --prompt 'A calm owl librarian, flat vector, navy and gold' \
  --reason 'Give the research agent a face'
```

Leave out `--prompt` and it draws from the agent's name and description.
Either way it asks for a square, centered picture with no text, so it still
reads as a 32 px circle. Over MCP the command is `generate_agent_avatar`; over
REST, `POST /v1/agents/{agent_id}/avatar/generate`.
