---
title: AGENTS.md
description: Project instructions resolved hierarchically from workspace files such as AGENTS.md and injected as the leading user message on every turn.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `agent_instructions` |
| **Category** | Core |
| **Features** | None |
| **Dependencies** | None |

Resolves project instruction files hierarchically, from the session filesystem root down to the working directory, and injects them as the leading user-role message on every turn. By default it reads `AGENTS.md`, Everruns' implementation of the [`AGENTS.md`](https://agents.md/) open convention. Configure `files` when an agent should also resolve another file such as `CLAUDE.md` at every hierarchy level.

Workspace files are untrusted third-party content, so they never enter the system prompt: harness safety instructions always take precedence, and file edits never invalidate the cache-stable system prefix.

## Tools

None. This capability only contributes conversation context, never system prompt.

## How it works

1. The agent receives a message.
2. Before the model call, the capability resolves the configured filenames from the filesystem root down to the working directory. A `docs/AGENTS.md` applies to work under `docs/`; deeper files override shallower ones on conflict. Files in sibling subtrees are never loaded.
3. The assembled block opens with a trust framing header and wraps each file in `<agent-instructions source="...">` tags, broadest scope first.
4. The block is injected as the leading user-role message: visible to the model, re-resolved every turn, and below system instructions in precedence.

Edits during a session apply on the next turn, with no restart. If a configured file doesn't exist, the agent operates normally.

## Prompt order

Every turn the model sees, top to bottom:

1. **System prompt**: harness safety instructions, tool guidance, role (cache-stable).
2. **Conversation context**: the resolved `AGENTS.md` hierarchy, broadest scope first.
3. **Conversation history and the latest user message**.

Explicit user instructions win over project files on conflict; system instructions always win over both.

## Config

```json
{
  "files": ["AGENTS.md", "CLAUDE.md"]
}
```

`files` is optional. When omitted, Everruns resolves `AGENTS.md` at every level from the root to the working directory. A config may list at most 16 files.

## Limits

- **32 KiB per file**. Longer content is truncated with a warning.
- **128 KiB total per turn**, which bounds hierarchy depth. Deeper files past the budget are omitted with an explicit note.
- At most 16 instruction files resolved per turn.
- Plain Markdown, no required sections.

## Notes

- Works with [File System](/capabilities/file-system/) tools, so an agent can update its own instructions.
- Missing configured files are ignored, with no error.

## Do something

- [Use AGENTS.md for project instructions](/how-to/use-agents-md/): full guide with examples.
- [Equip an agent with tools](/how-to/equip-agents-with-tools/): adding capabilities generally.

## See also

- [File System](/capabilities/file-system/): manage the AGENTS.md file.
- [Agent Skills](/capabilities/agent-skills/): another way to inject specialized instructions.
- [Capabilities Overview](/capabilities/)
