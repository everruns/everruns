---
title: AGENTS.md
description: The AGENTS.md capability injects project-level instructions from workspace files into the agent's system prompt on every turn.
---

The **AGENTS.md** capability reads project instruction files from the session workspace and injects them into the agent's system prompt at the start of every turn. By default it reads `AGENTS.md`, Everruns' implementation of the [`AGENTS.md`](https://agents.md/) open standard, an emerging convention backed by OpenAI, Google, Cursor, Sourcegraph, and the Linux Foundation.

When the capability is enabled:

- The agent reads `/workspace/AGENTS.md` on every turn by default.
- Agents can configure `files` to read additional workspace files such as `CLAUDE.md`.
- Edits during a session apply on the next turn (no restart).
- The content is prepended to the system prompt, before capability fragments and the agent's own prompt.
- If a configured file doesn't exist, the agent operates normally.

## Prompt order

When `AGENTS.md` is present, the system prompt is composed top-to-bottom:

1. **Instruction file content**: project-level instructions
2. **Capability system prompt additions**: tool guidance
3. **Agent's base system prompt**: role and personality

Project context comes first, so downstream prompt fragments can build on it.

## Limits

- Content cap: **32 KiB per file**. Excess is silently truncated.
- Plain Markdown, no required sections.

## Compatibility

Everruns keeps `AGENTS.md` as the default. Configure the capability with `files` to read additional tool-specific files:

```json
{
  "files": ["AGENTS.md", "CLAUDE.md"]
}
```

## Do something

- [Use AGENTS.md for project instructions](/how-to/use-agents-md/), full guide with examples.
- [Equip an agent with tools](/how-to/equip-agents-with-tools/), adding capabilities generally.

## See also

- [AGENTS.md capability reference](/capabilities/agent-instructions/), config schema.
