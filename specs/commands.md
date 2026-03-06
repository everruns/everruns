# Commands System

Slash commands for session interaction, following patterns from Claude Code, Codex CLI, and GitHub Copilot.

## Design

Two command sources, unified under `CommandDescriptor`:

1. **System Commands** — from capabilities via `Capability::commands()`. Execute directly without LLM (session control).
2. **Skill Commands** — from skills marked `user-invocable: true` in SKILL.md frontmatter. Expand to prompt injection (LLM processes the skill instructions).

Commands are NOT tools. They either execute directly (system) or inject instructions into the conversation (skills).

## Types

See `crates/core/src/command.rs` for `CommandDescriptor`, `CommandSource`, `CommandArg`, `CommandResult`.

## Capability Trait Extension

`Capability::commands()` returns `Vec<CommandDescriptor>` (default: empty). Capabilities that provide commands override this method.

See `crates/core/src/capabilities/system_commands.rs` for the built-in system commands capability (`/clear`, `/status`, `/compact`, `/model`).

## Skill Invocability

SKILL.md frontmatter supports `user-invocable` (default: `true`). Skills with `user-invocable: false` provide context/tools but don't appear as slash commands.

For DB-backed skills, `user_invocable` is stored in the metadata JSON field (no schema migration needed). See `crates/server/src/services/skill.rs`.

## API

- `GET /v1/sessions/{session_id}/commands` — returns all available commands (system + invocable skills)
- `POST /v1/sessions/{session_id}/commands/{command_name}` — execute a command (placeholder)

See `crates/server/src/api/commands.rs` for route handlers.

## UI Integration

The UI fetches commands via the GET endpoint to populate command palette / autocomplete when the user types `/`. System commands execute immediately; skill commands expand the skill's instructions into the conversation.
