---
title: AGENTS.md
description: Dynamic project instructions from AGENTS.md in the session workspace
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

## Use Cases

- **Project conventions** — coding style, naming rules, architectural constraints
- **Build instructions** — how to compile, test, and deploy the project
- **Context injection** — repository structure, key files, domain-specific terminology
- **Dynamic guidance** — update instructions mid-session by modifying the file

## Example

Upload an `AGENTS.md` to the session workspace:

```markdown
# Project: acme-api

## Stack
- Python 3.12, FastAPI, SQLAlchemy
- PostgreSQL 16, Redis 7

## Conventions
- Use snake_case for all Python identifiers
- All endpoints return JSON with `{ data, error }` envelope
- Tests go in `tests/` mirroring `src/` structure

## Build
- `pip install -e ".[dev]"` to install
- `pytest` to run tests
- `ruff check .` to lint
```

The agent will follow these instructions for every turn in the session.

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
