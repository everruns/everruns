---
title: Use AGENTS.md for project instructions
description: Inject project-level context, coding style, build commands, architecture notes, into an agent's leading user message by enabling the AGENTS.md capability.
---

`AGENTS.md` is an emerging open standard for providing project-level instructions to AI agents, backed by OpenAI, Google, Cursor, Sourcegraph, and others. Everruns ships it as the default file for its built-in agent instructions capability, which re-reads configured files on every turn.

## Enable the capability

```bash
curl -X PATCH http://localhost:9300/api/v1/agents/$AGENT_ID \
  -H "Content-Type: application/json" \
  -d '{
    "capabilities": [
      { "ref": "agent_instructions" },
      { "ref": "session_file_system" }
    ]
  }'
```

`session_file_system` isn't required, but pairing it with `agent_instructions` lets the agent edit `AGENTS.md` itself.

To also read another instruction file, configure `files` on the capability:

```bash
curl -X PATCH http://localhost:9300/api/v1/agents/$AGENT_ID \
  -H "Content-Type: application/json" \
  -d '{
    "capabilities": [
      {
        "ref": "agent_instructions",
        "config": {
          "files": ["AGENTS.md", "CLAUDE.md"]
        }
      },
      { "ref": "session_file_system" }
    ]
  }'
```

## Write the file

Drop a plain Markdown file at `AGENTS.md` in the session workspace root for repo-wide rules. Add nested files (for example `docs/AGENTS.md`) for subdirectory-scoped rules — deeper files override shallower ones on conflict, and sibling subtrees never see each other's files:

```markdown
## Project: Acme API

REST API built with Rust + Axum. PostgreSQL for storage.

## Style

- snake_case for variables and functions
- PascalCase for types
- Keep functions under 50 lines

## Build & Test

    cargo build
    cargo test --all-features
    cargo clippy -- -D warnings

## Architecture

- `src/api/` — HTTP handlers
- `src/domain/` — Business logic
- `src/db/` — Database queries

## Commits

Use conventional commits: `feat(scope): description`
```

There are no required sections. Write whatever a new contributor would need to know.

## How it lands in the prompt

Every turn the model sees, top-to-bottom:

1. **System prompt**: harness safety instructions, tool guidance, role.
2. **Conversation context**: your resolved `AGENTS.md` hierarchy, broadest scope first.
3. **Conversation history and your message**.

Project files never enter the system prompt — system instructions always win on conflict, and your explicit message wins over project files.

## Limits and dynamics

- Content is capped at **32 KiB** (32,768 bytes) per file (excess truncated with a warning), plus a **128 KiB total budget** per turn across the hierarchy.
- Configured files are resolved from the filesystem root down to the working directory on every turn. Edits during a session apply on the next turn, no restart needed.
- If a configured file doesn't exist, the agent operates normally without it.

## Other tools' instruction files

Everruns reads `AGENTS.md` by default at every hierarchy level. Add other file names to `files` when an agent should also resolve `CLAUDE.md`, `.cursorrules`, or `.github/copilot-instructions.md` per level.

## See also

- [AGENTS.md capability reference](/capabilities/agent-instructions/)
- [Equip an agent with tools](/how-to/equip-agents-with-tools/), adding capabilities in general.
