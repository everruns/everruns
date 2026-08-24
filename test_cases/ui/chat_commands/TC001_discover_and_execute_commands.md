# TC001: Discover and Execute Chat Commands

## Description

Verify slash-command discovery, selection, execution, persistence, and failure behavior in global Chat and a normal session Chat.

## Preconditions

- DB-backed stack running with the canonical startup script
- `AUTH_MODE=none` or an authenticated user
- Global Chat enabled
- Platform Chat and Generic harnesses available
- One active user-invocable skill

## Test Data

| Input | Purpose |
|-------|---------|
| `/` | Open the command menu |
| `/b` | Filter system commands |
| `/btw What was my last request?` | Execute a required-argument system command |
| `/ui-review Review this response` | Send an invocable skill command |
| `ordinary message` | Verify non-command fallback |

## Steps

1. Open `/chats`, start a **New chat** on the built-in **Platform Chat** harness, record the session ID from the thread URL (`/chats/{sessionId}`), and inspect `GET /v1/sessions/{id}/commands`.
2. Type `/`, then `/b`. Verify the accessible menu, filtering, and selected option.
3. Select `/btw` with the keyboard. Verify it fills the composer until its required argument is supplied.
4. Submit `/btw` with an argument. Verify the command endpoint is used, the result is dismissible, Escape restores composer focus, and message history count is unchanged.
5. Reload the thread URL. Verify the same owned session is reused and an ordinary message succeeds.
6. Open a Generic `/sessions/{id}/chat` page, type `/`, and select the invocable skill with the pointer. Verify touch-sized/pointer selection restores focus and fills the composer.
7. Send the filled skill command. Verify it uses the ordinary message endpoint and is persisted.
8. Repeat the menu checks at desktop and mobile widths; verify the menu remains visible and scrollable.
9. Fail the command-discovery request, then send an ordinary message. Verify the composer stays usable and does not advertise unavailable commands.
10. Replace the resolved session while a draft is present. Verify stale draft, command result, and command query state do not carry into the new session.

## Expected Result

| Check | Expected |
|-------|----------|
| Session binding | Discovery and execution use the currently resolved session ID |
| Global Chat ownership | Current owner can send; another user cannot discover or execute its commands |
| Menu | Visible above the composer, accessible as a listbox, keyboard/pointer/touch operable |
| Required arguments | Selection fills the command; incomplete submission does not execute |
| System command | Uses `/commands/execute`; result is dismissible and not persisted |
| Skill command | Fills then sends through the ordinary message endpoint and persists |
| Empty/error | Ordinary messages remain usable; unavailable commands are not advertised |
| Replacement | Stale composer and command state are cleared |
| Responsive layout | Menu remains usable at desktop and mobile widths |
