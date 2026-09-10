---
title: AGENTS.md
description: The AGENTS.md capability resolves project-level instructions from workspace files hierarchically and injects them as the leading user message on every turn.
---

The **AGENTS.md** capability resolves project instruction files hierarchically — from the session filesystem root down to the working directory — and injects them as the leading user-role message on every turn. By default it reads `AGENTS.md`, Everruns' implementation of the [`AGENTS.md`](https://agents.md/) open standard, an emerging convention backed by OpenAI, Google, Cursor, Sourcegraph, and the Linux Foundation.

Workspace files are untrusted third-party content, so they never enter the system prompt: harness safety instructions always take precedence, and file edits never invalidate the cache-stable system prefix.

When the capability is enabled:

- The agent resolves `AGENTS.md` from the filesystem root down to the working directory on every turn by default.
- A `docs/AGENTS.md` applies to work under `docs/`; deeper files override shallower ones on conflict. Files in sibling subtrees are never loaded.
- Agents can configure `files` to resolve additional filenames such as `CLAUDE.md` at every hierarchy level.
- Edits during a session apply on the next turn (no restart).
- The assembled block opens with a trust framing header and renders each file in `<agent-instructions source="...">` sections as the first user message — never as system prompt.
- If a configured file doesn't exist, the agent operates normally.

## Prompt order

Every turn the model sees, top-to-bottom:

1. **System prompt**: harness safety instructions, tool guidance, role (cache-stable).
2. **Conversation context**: the resolved `AGENTS.md` hierarchy, broadest scope first — model-visible, re-resolved every turn, but below system instructions in precedence.
3. **Conversation history and the latest user message**.

Explicit user instructions win over project files on conflict; system instructions always win over both.

## Limits

- Content cap: **32 KiB per file** (excess truncated with a warning), plus a **128 KiB total budget** per turn binding hierarchy depth — deeper files past the budget are omitted with an explicit note.
- Plain Markdown, no required sections.
- At most 16 instruction files resolved per turn.

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
