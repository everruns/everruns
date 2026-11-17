---
title: everruns agent-skill
description: "The Everruns skill for coding agents, as shipped with this CLI. CLI reference for everruns agent-skill: usage, flags and examples."
sidebar:
  label: agent-skill
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

The Everruns skill for coding agents, as shipped with this CLI.

| Command | What it does |
|---|---|
| [`agent-skill install`](#agent-skill-install) | Write the skill into this project's .agents/skills and .claude/skills. |
| [`agent-skill show`](#agent-skill-show) | Print the skill, or one of its files such as references/install.md. |
| [`agent-skill files`](#agent-skill-files) | List the skill's files. |

## agent-skill install

Write the skill into this project's .agents/skills and .claude/skills.

```bash
everruns agent-skill install [OPTIONS]
```

| Flag | Description |
|---|---|
| `--global` | Install for your user (~/.agents/skills and ~/.claude/skills) instead of this project. |
| `--dir <DIR>` | Write into this skills directory only, as <DIR>/everruns. |


## agent-skill show

Print the skill, or one of its files such as references/install.md.

```bash
everruns agent-skill show [FILE]
```

| Flag | Description |
|---|---|
| `<FILE>` | File inside the skill (default SKILL.md) |


## agent-skill files

List the skill's files.

```bash
everruns agent-skill files
```

