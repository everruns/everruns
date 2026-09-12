---
title: AGENTS.md
description: Dynamic project instructions loaded from configured files in the session workspace. Agents inherit coding style, tool preferences, and workflow rules automatically.
---

| | |
|---|---|
| **ID** | `agent_instructions` |
| **Category** | Core |
| **Features** | None |
| **Dependencies** | None |

Reads project instruction files hierarchically from the session workspace and injects them as the leading user message on every turn. By default it reads `AGENTS.md`. Configure `files` when an agent should also resolve another file such as `CLAUDE.md` at every hierarchy level.

## Tools

None, this capability only contributes conversation context (never system prompt).

## How It Works

1. Agent sends a message
2. Before processing, the system resolves configured filenames from the filesystem root down to the working directory
3. Each file is wrapped in `<agent-instructions source="...">` XML tags, broadest scope first, behind a trust framing header
4. Injected as the leading user-role message — model-visible, re-resolved every turn, below system instructions in precedence

## Config

```json
{
  "files": ["AGENTS.md", "CLAUDE.md"]
}
```

`files` is optional. When omitted, Everruns reads only `/workspace/AGENTS.md`.

## Notes

- Default file name: `AGENTS.md` (plain Markdown, max 32 KiB per file, 128 KiB total per turn)
- Hierarchy: root to working directory; deeper files win, siblings out of scope
- Re-resolved every turn, edits take effect immediately
- Missing configured files are ignored (no error)
- Works with [File System](/capabilities/file-system/) tools to update instructions dynamically

## See Also

- [AGENTS.md feature guide](/features/agent-instructions/), detailed documentation
- [File System](/capabilities/file-system/), manage the AGENTS.md file
- [Agent Skills](/capabilities/agent-skills/), another way to inject specialized instructions
- [Capabilities Overview](/capabilities/)
