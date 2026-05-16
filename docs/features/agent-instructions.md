---
title: AGENTS.md
description: The AGENTS.md capability injects project-level instructions from a workspace file into the agent's system prompt on every turn.
---

The **AGENTS.md** capability reads an `AGENTS.md` file from the session workspace and injects it into the agent's system prompt at the start of every turn. It's Everruns' implementation of the [`AGENTS.md`](https://agents.md/) open standard — an emerging convention backed by OpenAI, Google, Cursor, Sourcegraph, and the Linux Foundation.

When the capability is enabled:

- The agent reads `/workspace/AGENTS.md` on every turn.
- Edits during a session apply on the next turn (no restart).
- The content is prepended to the system prompt, before capability fragments and the agent's own prompt.
- If the file doesn't exist, the agent operates normally.

## Prompt order

When `AGENTS.md` is present, the system prompt is composed top-to-bottom:

1. **AGENTS.md content** — project-level instructions
2. **Capability system prompt additions** — tool guidance
3. **Agent's base system prompt** — role and personality

Project context comes first, so downstream prompt fragments can build on it.

## Limits

- Content cap: **32 KiB**. Excess is silently truncated.
- Plain Markdown — no required sections.

## Compatibility

Everruns reads only `AGENTS.md`. Tool-specific files like `.cursorrules`, `CLAUDE.md`, and `.github/copilot-instructions.md` are not read. If you maintain multiple, symlink or copy into `AGENTS.md`.

## Do something

- [Use AGENTS.md for project instructions](/how-to/use-agents-md/) — full guide with examples.
- [Equip an agent with tools](/how-to/equip-agents-with-tools/) — adding capabilities generally.

## See also

- [AGENTS.md capability reference](/capabilities/agent-instructions/) — config schema.
