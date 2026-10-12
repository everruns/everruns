---
title: One everruns command line for the CLI, MCP and agent shells
description: Every Everruns operation now has one noun-verb spelling with --help that works the same in the everruns CLI, the /mcp endpoint and an agent's shell.
date: 2026-09-17
kind: feature
significance: major
products: [platform, cloud, cli, framework]
surfaces: [cli, agent, code]
release: "0.29.0"
links:
  docs: [/features/cli/, /reference/cli/]
  prs: [everruns/everruns#3680, everruns/everruns#3490, everruns/everruns#3496]
---

`everruns <noun> <verb> --flags` is now a single grammar. The external CLI, the
`/mcp` endpoint and a session's shell render it from one declaration, so the CLI
grows from 82 commands to 309 across 31 nouns, and `--help` reaches every one of
them. Flat names such as `list_agents` keep working as aliases.

```bash
everruns agents list
everruns sessions --help
```

**In code:** a Framework application exposing its own command tree can name its
root token with `CliCommandSource::root` instead of `everruns`.

A Platform Chat v2 harness built on this command line is available behind an
organization opt-in flag. See [CLI](/features/cli/) and
[CLI reference](/reference/cli/). [#3680](https://github.com/everruns/everruns/pull/3680), [#3490](https://github.com/everruns/everruns/pull/3490), [#3496](https://github.com/everruns/everruns/pull/3496)
