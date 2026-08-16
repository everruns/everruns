---
type: Specification
title: "Commands System"
description: "Slash commands system."
tags:
  - everruns
  - project
---
# Commands System

Slash commands for session interaction, following patterns from Claude Code, Codex CLI, and GitHub Copilot.

## Design

Two command sources, unified under `CommandDescriptor`:

1. **System Commands**: from capabilities via `Capability::commands()`. Execute directly without turning the command text into a persisted chat message.
2. **Skill Commands**: from skills marked `user-invocable: true` in SKILL.md frontmatter. Expand to prompt injection (LLM processes the skill instructions).

Commands are NOT tools. They either execute directly (system) or inject instructions into the conversation (skills).

## Types

See `crates/core/src/command.rs` for `CommandDescriptor`, `CommandSource`, `CommandArg`, `CommandResult`.

## Capability Trait Extension

`Capability::commands()` returns `Vec<CommandDescriptor>` (default: empty). Capabilities that provide commands override this method. `Capability::execute_command()` executes a declared command; capabilities that declare commands must override it.

See `crates/builtins/src/btw.rs` for the built-in `/btw` capability.

## Command Host (EVE-543)

Some commands need an out-of-band LLM call over the session's assembled context, `/btw` is the canonical case. Historically the server intercepted `/btw` with a bespoke executor before capability dispatch, so every other host (`InProcessRuntime` embedders) either got a broken `/btw` or vendored a reimplementation. The contract is instead extended so this class of command is implementable once, inside the capability.

### Contract

`CommandExecutionContext` carries, in addition to `session_id`, a host handle implementing `CommandHost` with two facilities:

1. **Turn-context access**: `turn_context()` assembles the same merged view a main turn would see (capability message filters applied, merged harness/agent/session system prompt, resolved model identity). The returned view is credential-free: it exposes the model name and provider type for error classification but never API keys or base URLs.
2. **Session completion**: `completion(request)` runs a tool-less completion against the session's resolved model (or a per-invocation `Controls` model override, resolved through the same org-scoped store as a main turn). The capability supplies the system prompt stack and core `Message`s; the host owns provider conversion (image resolution, external-actor prefixes, dangling-tool-call patching), driver creation, and credentials. Completion errors carry the resolved provider/model identity so callers can classify them into stable user-facing error codes. `completion_stream(request)` is the streaming variant for progressive output, same request semantics and guarantees, returning provider stream events plus the classification context for mid-stream errors. It defaults to unsupported so commands fall back to `completion` on hosts that cannot stream; the store-backed host supports it.

Command execution stays out-of-band by construction: the host facilities persist nothing (no messages, no events). A future facility for explicit persistence can be added if a command needs it.

Decisions:

- Two composable facilities rather than one "run /btw" helper, so future commands can reshape the context (different system prompt, truncated history) without new contract surface.
- The completion facility is deliberately tool-less, mirroring `UtilityLlmService`'s request conventions, but it is a distinct abstraction: the utility LLM is a deployment-fixed model with host credentials; session completion uses the session's resolved, org-scoped model.
- Credentials never cross the trait boundary (same posture as TM-LLM-021 for the utility LLM).
- Hosts that cannot provide the facilities pass a disabled host whose methods fail with a clear "host does not support context-aware commands" error, so misconfiguration surfaces at invocation, not silently.

### One implementation, three hosts

Core provides only the credential-free `CommandHost` contract. The concrete
`everruns_host::StoreCommandHost` is built from the host's store traits
(`HarnessStore`, `AgentStore`, `SessionStore`, `MessageRetriever`,
`ProviderStore`, optional `ImageResolver`/`SessionFileSystem`) plus the
capability and driver registries. It reuses host-owned `inspect_turn_context`
and the reason-path message-building helpers, so the side answer sees exactly
what a main turn would. `CommandTurnContext` carries only `session_id`, never a
session record.

- **Server (full and dev mode)**: `SessionCommandService` wires the store-backed host from its worker adapters and dispatches every system command through `Capability::execute_command`; the bespoke `/btw` executor and its command-name special case are gone.
- **In-process runtime**: `InProcessRuntime::execute_command` wires the same host from its runtime stores, so embedders get working context-aware commands just by registering the capability.

Current built-in system command:

- `/btw <question>`, ask an ephemeral side question about the current session. The answer:
  - sees the current session prompt and conversation history
  - has no tool access
  - does not append a message to the main chat history
  - is shown in the UI as a dismissible overlay

`/btw` is enabled by the Generic and Platform Chat harnesses via the `btw` capability.

## Skill Invocability

SKILL.md frontmatter supports `user-invocable` (default: `true`). Skills with `user-invocable: false` provide context/tools but don't appear as slash commands.

For DB-backed skills, `user_invocable` is stored in the metadata JSON field (no schema migration needed). See `crates/server/src/domains/skills/commands.rs`.

## API

- `GET /v1/sessions/{session_id}/commands`, returns all available commands (system + invocable skills)
- `POST /v1/sessions/{session_id}/commands/execute`, executes a system command without persisting a chat message

See `crates/server/src/api/commands.rs` for route handler.

## UI Integration

- `apps/ui/src/components/chat/command-autocomplete.tsx`, autocomplete popup component
- `apps/ui/src/hooks/use-commands.ts`, `useSessionCommands` React Query hook
- `apps/ui/src/lib/api/commands.ts`, API client

The UI fetches commands via the GET endpoint to populate autocomplete when the user types `/` in the chat input. Keyboard navigation (arrows, Enter/Tab, Escape) is supported.

- Skill commands fill the input with `/{name} ` for the user to send.
- System commands execute via the POST endpoint instead of being sent as chat messages.
- Commands with required args, such as `/btw`, fill `/{name} ` on selection so the user can type the argument before execution.
