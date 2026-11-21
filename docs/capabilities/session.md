---
title: Session
description: Inspect and update the current session's metadata, including its ID, title, agent name, locale, and usage.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `session` |
| **Category** | Session |
| **Features** | None |
| **Dependencies** | None |

Tools to read and update session metadata like title and agent information.

## Automatic titles

For naming outside the agent loop, enable [Automatic session titles](/capabilities/automatic-session-titles/) and leave `auto_title` off. Explicit tool writes take priority over automatic titles, including same-text writes.

Agent-directed title maintenance is opt-in. Set `auto_title` to `true` in the
capability configuration to have the agent create a concise 3–7 word title
before handling the first substantive request. The title is a required pre-work
update. The agent updates it later, also before other work or a response, only
when the conversation's primary theme materially changes, not for minor
follow-ups or subtopics. Title writes update session metadata and do not count
as project or workspace file changes.

Title changes emit `session.title.updated` with the previous and new title. A
repeated write of the current title is a no-op and emits no event.

## Tools

### `get_session_info`

Get current session metadata.

Returns: session ID, title, locale, agent name, token usage (input, output,
cache read, cache creation, and total tokens) when available, and `spend`: what
this session has spent so far in USD.

`spend` adds up:

- **Model calls** in turns, with a split by input, cached input, cache writes
  and output, and per model. A cost the provider reports (OpenRouter) is used
  as is; other calls are priced from the model's price table.
- **Background calls** the platform made for the session, such as titles and
  compaction.
- **Images** from `generate_image` and `edit_image`, priced from their usage.
- **Sandboxes**: running and stopped time, priced by your organization's price
  list when it has a `sandbox.time` rate, otherwise by the provider's published
  rate for the sandbox's size (Daytona, E2B, Modal).
- **Subagents** the session started, each with its own whole spend.

It also returns `current_turn_usd` (this turn so far), `wasted_usd` (calls in
turns that failed), `basis` (`provider_reported`, `price_list` or
`list_price`; the weakest part wins) and `partial` when something had no price.
Paid tools on your own keys (search, browser), knowledge indexing and
evaluations are not included.

The figure covers only this session and its subagents. It never includes
organization totals, budgets, credits or your price list, so an agent can
quote it, for example in a pull request comment. `spend` is `null` on hosts
that cannot price it, such as the embedded framework.

### `write_session_title`

Update the session title.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `title` | string | yes | New session title |

## See Also

- [Storage](/capabilities/session-storage/), persist data within the session
- [Schedules](/capabilities/session-schedules/), schedule future tasks
- [Capabilities Overview](/capabilities/)
