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

Reads project instruction files from the session workspace and injects their content into the system prompt. By default it reads `AGENTS.md`. Configure `files` when an agent should also read another file such as `CLAUDE.md`.

## Tools

None, this capability only contributes to the system prompt.

## How It Works

1. Agent sends a message
2. Before processing, the system reads configured files from the session filesystem
3. Each file is wrapped in `<agent-instructions source="...">` XML tags
4. Injected at the beginning of the system prompt (before other capability prompts)

## Config

```json
{
  "files": ["AGENTS.md", "CLAUDE.md"]
}
```

`files` is optional. When omitted, Everruns reads only `/workspace/AGENTS.md`.

## Notes

- Default file path: `/workspace/AGENTS.md` (plain Markdown, max 32 KiB per file)
- Re-read every turn, edits take effect immediately
- Missing configured files are ignored (no error)
- Works with [File System](/capabilities/file-system/) tools to update instructions dynamically

## See Also

- [AGENTS.md feature guide](/features/agent-instructions/), detailed documentation
- [File System](/capabilities/file-system/), manage the AGENTS.md file
- [Agent Skills](/capabilities/agent-skills/), another way to inject specialized instructions
- [Capabilities Overview](/capabilities/)
