---
title: "Connect your coding agent from the Everruns UI"
description: "Copy one setup prompt to connect Claude Code, Codex, Cursor or Gemini CLI, with provider setup and OAuth connection management kept explicit."
date: 2026-10-16
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/getting-started/use-in-ai-tools/]
---

![The coding-agent connection dialog with a setup prompt and manual MCP configuration](./connection-dialog@2x.png)

Choose **Connect your coding agent** in the user menu, after organisation setup,
or on an empty Agents page. Copy the prompt into Claude Code, Codex, Cursor or
Gemini CLI to follow the maintained plugin instructions and sign in. Self-hosted
prompts carry your deployment's MCP endpoint.

**Check connections** opens your OAuth approvals, with refresh and revoke controls.
Model-provider setup remains separate: after skipping it, onboarding keeps an
**Add a model provider** action visible before running agents or Platform Chat.

See [Use in AI Tools](/getting-started/use-in-ai-tools/) for manual installation.
