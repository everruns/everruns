---
title: AGENTS.md
description: Dynamic project instructions loaded from AGENTS.md files in the session workspace. Agents inherit coding style, tool preferences, and workflow rules automatically.
---

| | |
|---|---|
| **ID** | `agent_instructions` |
| **Category** | Configuration |
| **Features** | None |
| **Dependencies** | None |

Reads `AGENTS.md` from the session workspace and injects its content into the system prompt. The file is re-read on every conversation turn, so changes are picked up automatically without restarting the session.

## Tools

None — this capability only contributes to the system prompt.

## How It Works

1. Agent sends a message
2. Before processing, the system reads `/workspace/AGENTS.md` from the session filesystem
3. Content is wrapped in `<agent-instructions source="AGENTS.md">` XML tags
4. Injected at the beginning of the system prompt (before other capability prompts)

## Notes

- File path: `/workspace/AGENTS.md` (plain Markdown, max 32 KiB)
- Re-read every turn — edits take effect immediately
- If the file doesn't exist, no instructions are added (no error)
- Works with [File System](/capabilities/file-system/) tools to update instructions dynamically

## See Also

- [AGENTS.md feature guide](/features/agent-instructions/) — detailed documentation
- [File System](/capabilities/file-system/) — manage the AGENTS.md file
- [Agent Skills](/capabilities/agent-skills/) — another way to inject specialized instructions
- [Capabilities Overview](/capabilities/)
