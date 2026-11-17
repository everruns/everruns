---
title: One Everruns skill for every coding agent
description: The Everruns skill now covers the Framework, self-hosting and agent work in one install, from npx, the plugin, every MCP server or the everruns CLI.
date: 2026-11-17
kind: improvement
significance: notable
products: [platform, framework, cli, cloud]
surfaces: [agent, cli]
links:
  docs: [/getting-started/use-in-ai-tools/, /reference/cli/agent-skill/]
---

Coding agents now learn Everruns from one skill instead of fourteen. Its short
`SKILL.md` maps each task to a guide in `references/`: building with the
Framework, installing Everruns, the CLI and MCP, and building, running,
debugging, evaluating and shipping agents. The agent reads only the guides a
task needs.

```bash
npx skills add everruns/everruns
```

The Everruns plugin carries the same skill, and every Everruns MCP server
serves it as `skill://everruns/SKILL.md`, matching that server's version.
[`everruns agent-skill install`](/reference/cli/agent-skill/) writes the copy
that matches your CLI into the project. The per-task skill names such as
`everruns-cli` and `everruns-mcp` are gone; their guides now live under
`references/`. See [Use in AI Tools](/getting-started/use-in-ai-tools/).
