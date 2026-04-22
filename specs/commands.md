# Commands System

Slash commands for session interaction, following patterns from Claude Code, Codex CLI, and GitHub Copilot.

## Design

Two command sources, unified under `CommandDescriptor`:

1. **System Commands** — from capabilities via `Capability::commands()`. Execute directly without turning the command text into a persisted chat message.
2. **Skill Commands** — from skills marked `user-invocable: true` in SKILL.md frontmatter. Expand to prompt injection (LLM processes the skill instructions).

Commands are NOT tools. They either execute directly (system) or inject instructions into the conversation (skills).

## Types

See `crates/core/src/command.rs` for `CommandDescriptor`, `CommandSource`, `CommandArg`, `CommandResult`.

## Capability Trait Extension

`Capability::commands()` returns `Vec<CommandDescriptor>` (default: empty). Capabilities that provide commands override this method.

See `crates/core/src/capabilities/btw.rs` for the built-in `/btw` capability.

Current built-in system command:

- `/btw <question>` — ask an ephemeral side question about the current session. The answer:
  - sees the current session prompt and conversation history
  - has no tool access
  - does not append a message to the main chat history
  - is shown in the UI as a dismissible overlay

`/btw` is enabled by the Generic harness via the `btw` capability.

## Skill Invocability

SKILL.md frontmatter supports `user-invocable` (default: `true`). Skills with `user-invocable: false` provide context/tools but don't appear as slash commands.

For DB-backed skills, `user_invocable` is stored in the metadata JSON field (no schema migration needed). See `crates/server/src/services/skill.rs`.

## API

- `GET /v1/sessions/{session_id}/commands` — returns all available commands (system + invocable skills)
- `POST /v1/sessions/{session_id}/commands/execute` — executes a system command without persisting a chat message

See `crates/server/src/api/commands.rs` for route handler.

## UI Integration

- `apps/ui/src/components/chat/command-autocomplete.tsx` — autocomplete popup component
- `apps/ui/src/hooks/use-commands.ts` — `useSessionCommands` React Query hook
- `apps/ui/src/lib/api/commands.ts` — API client

The UI fetches commands via the GET endpoint to populate autocomplete when the user types `/` in the chat input. Keyboard navigation (arrows, Enter/Tab, Escape) is supported.

- Skill commands fill the input with `/{name} ` for the user to send.
- System commands execute via the POST endpoint instead of being sent as chat messages.
- Commands with required args, such as `/btw`, fill `/{name} ` on selection so the user can type the argument before execution.
