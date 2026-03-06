# Commands System

Slash commands for session interaction, following patterns from Claude Code, Codex CLI, and GitHub Copilot.

## Design

Two command sources, unified under `CommandDescriptor`:

1. **System Commands** — from capabilities via `Capability::commands()`. Execute directly without LLM (session control). No system commands are registered yet; the `SystemCommandsCapability` is the extension point.
2. **Skill Commands** — from skills marked `user-invocable: true` in SKILL.md frontmatter. Expand to prompt injection (LLM processes the skill instructions).

Commands are NOT tools. They either execute directly (system) or inject instructions into the conversation (skills).

## Types

See `crates/core/src/command.rs` for `CommandDescriptor`, `CommandSource`, `CommandArg`, `CommandResult`.

## Capability Trait Extension

`Capability::commands()` returns `Vec<CommandDescriptor>` (default: empty). Capabilities that provide commands override this method.

See `crates/core/src/capabilities/system_commands.rs` for the system commands capability (currently empty; add commands as handlers are implemented).

## Skill Invocability

SKILL.md frontmatter supports `user-invocable` (default: `true`). Skills with `user-invocable: false` provide context/tools but don't appear as slash commands.

For DB-backed skills, `user_invocable` is stored in the metadata JSON field (no schema migration needed). See `crates/server/src/services/skill.rs`.

## API

- `GET /v1/sessions/{session_id}/commands` — returns all available commands (system + invocable skills)

See `crates/server/src/api/commands.rs` for route handler.

## UI Integration

- `apps/ui/src/components/chat/command-autocomplete.tsx` — autocomplete popup component
- `apps/ui/src/hooks/use-commands.ts` — `useSessionCommands` React Query hook
- `apps/ui/src/lib/api/commands.ts` — API client

The UI fetches commands via the GET endpoint to populate autocomplete when the user types `/` in the chat input. Keyboard navigation (arrows, Enter/Tab, Escape) is supported. System commands would execute immediately; skill commands fill the input with `/{name}` for the user to send.
