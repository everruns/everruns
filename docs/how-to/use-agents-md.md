---
title: Use AGENTS.md for project instructions
description: Inject project-level context — coding style, build commands, architecture notes — into an agent's system prompt by enabling the AGENTS.md capability.
---

`AGENTS.md` is an emerging open standard for providing project-level instructions to AI agents, backed by OpenAI, Google, Cursor, Sourcegraph, and others. Everruns ships it as a built-in capability that re-reads the file on every turn.

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

## Write the file

Drop a plain Markdown file at `/workspace/AGENTS.md` in the session:

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

When the capability is enabled, the system prompt is composed top-to-bottom:

1. **AGENTS.md content** — project-level instructions.
2. **Capability system prompt additions** — tool guidance.
3. **Agent's base system prompt** — the agent's role.

Project context comes first, so capability and agent prompts can reference it.

## Limits and dynamics

- Content is capped at **32 KiB** (32,768 bytes). Excess is silently truncated.
- The file is re-read on every turn. Edits during a session apply on the next turn — no restart needed.
- If the file doesn't exist, the agent operates normally without it.

## Other tools' instruction files

Everruns reads only `AGENTS.md`. Files like `.cursorrules`, `CLAUDE.md`, or `.github/copilot-instructions.md` aren't read. If you maintain several, symlink or copy them into `AGENTS.md` for Everruns.

## See also

- [AGENTS.md capability reference](/capabilities/agent-instructions/)
- [Equip an agent with tools](/how-to/equip-agents-with-tools/) — adding capabilities in general.
