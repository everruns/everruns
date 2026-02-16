---
title: AGENTS.md
description: Provide project-level instructions to agents via AGENTS.md
---

The **Agent Instructions** capability reads an `AGENTS.md` file from the session workspace and automatically includes it in the agent's system prompt on every turn. This gives you a simple, standard way to provide project-level context — coding conventions, style guides, build instructions, or any other guidance — to your agents.

## Overview

[`AGENTS.md`](https://agents.md/) is an emerging open standard for providing project-level instructions to AI coding agents. It is backed by organizations including OpenAI, Google, Cursor, Sourcegraph, and the Linux Foundation.

Everruns implements `AGENTS.md` as a built-in capability. When enabled:

- The agent reads `/workspace/AGENTS.md` from the session filesystem on every turn
- Changes to the file are picked up automatically (no restart needed)
- Content is prepended to the system prompt, before capability additions and the agent's own prompt
- If the file doesn't exist, the agent operates normally without it

## Quick Start

1. **Enable the capability** on your agent:

```bash
curl -X PATCH http://localhost:9300/api/v1/agents/{agent_id} \
  -H "Content-Type: application/json" \
  -d '{ "capabilities": [{ "ref": "agent_instructions" }] }'
```

2. **Write an AGENTS.md file** to your session workspace:

```bash
# Via the file system tools during a session
write_file("/workspace/AGENTS.md", "## Style\n\nUse snake_case for all variables.\nPrefer composition over inheritance.\n")
```

3. The agent will automatically read and follow the instructions on every turn.

## What to Put in AGENTS.md

`AGENTS.md` is plain Markdown. There are no required sections or special syntax. Common contents include:

- **Project overview** — what the project does, key concepts
- **Coding conventions** — naming, formatting, patterns to follow or avoid
- **Build and test commands** — how to build, run tests, lint
- **Architecture notes** — key directories, data flow, important modules
- **PR and commit conventions** — branching strategy, commit message format
- **Security considerations** — auth patterns, data handling rules

### Example

```markdown
## Project: Acme API

REST API built with Rust + Axum. PostgreSQL for storage.

## Style

- snake_case for variables and functions
- PascalCase for types
- Keep functions under 50 lines
- Write tests for all public functions

## Build & Test

    cargo build
    cargo test --all-features
    cargo clippy -- -D warnings

## Architecture

- `src/api/` — HTTP handlers
- `src/domain/` — Business logic
- `src/db/` — Database queries
- `src/models/` — Data types

## Commits

Use conventional commits: `feat(scope): description`
```

## How It Works

### System Prompt Order

When `AGENTS.md` is present, the system prompt is composed in this order (top to bottom):

1. **AGENTS.md content** — project-level instructions
2. **Capability system prompt additions** — tool guidance from enabled capabilities
3. **Agent's base system prompt** — the agent's own role and personality

This ensures project context comes first, then capability-specific guidance, then agent behavior.

### Size Limit

`AGENTS.md` content is capped at **32 KiB** (32,768 bytes). Content exceeding this limit is silently truncated. This matches the convention used by other tools like OpenAI Codex.

### Dynamic Updates

The file is re-read on every LLM turn. You can edit `AGENTS.md` at any point during a session and the agent will use the updated content on the next turn. No need to restart the session or re-enable the capability.

## Managing via UI

1. Navigate to the Agent detail page
2. Find **Capabilities** in the sidebar
3. Enable **Agent Instructions**
4. Save changes

Then write an `AGENTS.md` file to the session workspace using the file tools or bash.

## Managing via API

Enable the capability:

```bash
curl -X PATCH http://localhost:9300/api/v1/agents/{agent_id} \
  -H "Content-Type: application/json" \
  -d '{
    "capabilities": [
      { "ref": "agent_instructions" },
      { "ref": "session_file_system" },
      { "ref": "current_time" }
    ]
  }'
```

The `session_file_system` capability is recommended (but not required) alongside `agent_instructions` — it provides the file tools needed to create and edit `AGENTS.md` within the session.

## Compatibility

Everruns reads only `AGENTS.md`. Other tool-specific files (`.cursorrules`, `CLAUDE.md`, `.github/copilot-instructions.md`) are not read. If you maintain multiple instruction files, you can symlink or copy the content into `AGENTS.md`.
