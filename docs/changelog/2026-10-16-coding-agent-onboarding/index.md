---
title: "Connect your coding agent from the Everruns UI"
description: "Copy one setup prompt for Claude Code, Codex, Cursor or Gemini CLI, with deployment-aware instructions and model-provider setup in its existing flow."
date: 2026-10-16
cloud: 2026-10-17
kind: improvement
significance: minor
products: [platform, cloud]
release: "0.49.0"
surfaces: [ui]
links:
  docs: [/getting-started/use-in-ai-tools/]
  prs: [everruns/everruns#4771, everruns/everruns#4782]
---

![Connect your coding agent with one setup prompt and recognizable agent icons](./connection-dialog@2x.png)

Choose **Connect your coding agent** from the user menu, the Agents first-use
state or after organisation setup. Copy the prompt into Claude Code, Codex,
Cursor or Gemini CLI to follow the maintained setup instructions. Agent icons
identify compatible tools; everyone uses the same prompt.

The prompt preserves your deployment's MCP endpoint and authentication mode.
Your agent follows its live documentation and discovery metadata, then guides
you through browser sign-in when needed. Model-provider setup stays in the
existing onboarding flow, with **Add a model provider** available after a skip.

See [Use in AI Tools](/getting-started/use-in-ai-tools/) for manual installation
and managing connected clients.
