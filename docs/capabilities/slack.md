---
title: Slack
description: "Act in the Slack conversation as the bot the workspace already invited: reactions, message updates, file uploads, and user lookups."
---

| | |
|---|---|
| **ID** | `slack` |
| **Category** | Integrations |
| **Features** | `slack_actions` |
| **Dependencies** | None |

An agent published to a [Slack Agent channel](/integrations/slack/) can reply in its thread. This
capability lets it do the rest — react to a message, rewrite one it posted, share a file, resolve a
user ID to a name — as the same bot the workspace invited.

No second credential. The tools resolve the channel's own bot token server-side, so there is
nothing extra to provision, scope, or rotate.

## Requirements

The tools only work in a session a Slack message created. An agent that has the capability enabled
but is running from the API, a schedule, or another channel has no Slack Agent channel to act as, and
every tool returns an error saying so rather than acting as some other channel's bot.

Where an agent carries two Slack Agent channels, each with its own bot, the tools act as the channel
that created the session.

Your Slack app needs the scope for each action you use: `reactions:write` for reactions,
`chat:write` for updates, `files:write` for uploads, and `users:read` for lookups. Slack answers a
missing scope with an error the agent sees.

## Tools

### `slack_add_reaction`

Add an emoji reaction to a message. The cheapest acknowledgement available — prefer it over posting
"working on it".

| Parameter | Type | Required | Description |
|---|---|---|---|
| `channel` | string | yes | Channel ID the message is in |
| `timestamp` | string | yes | The message's `ts` |
| `name` | string | yes | Emoji name without colons (e.g. `eyes`) |

Reacting with an emoji that is already there succeeds; the result says `already_reacted`.

### `slack_update_message`

Rewrite a message this bot posted. Use it to turn a status message into its result instead of
posting a second message.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `channel` | string | yes | Channel ID the message is in |
| `timestamp` | string | yes | The `ts` of the bot message to rewrite |
| `text` | string | yes | Replacement text; Markdown is rendered |

Only messages this bot posted can be updated.

### `slack_lookup_user`

Resolve a Slack user ID to that person's display name, real name, timezone, and whether they are a
bot.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `user_id` | string | yes | Slack user ID (the `<@U…>` mention form is accepted) |

Returns only those addressing fields. Email, phone, and title are not exposed to the agent.

### `slack_upload_file`

Share a file into the conversation. Use it for reports, diffs, and logs too long to read in a
message.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `channel` | string | yes | Channel ID to share into |
| `filename` | string | yes | Filename shown in Slack, including its extension |
| `content` | string | yes | The file's text content |
| `thread_ts` | string | no | Thread to share into; omit to post at channel level |
| `initial_comment` | string | no | Message posted alongside the file |

Content is capped at 8 MiB.

## Notes

- Posting to an arbitrary channel is deliberately not offered. The blast radius of "anywhere the bot
  is" is wider than "the thread that asked", and the reply path already answers in the thread.
- A retired or disabled channel stops acting immediately, even for a session it created earlier.
- Slack rate limits reach the agent with Slack's own retry advice rather than as a generic failure.
- The [Slack MCP server](/features/mcp/) stays supported for anything this does not cover. This
  removes the second credential for the common cases; it does not replace MCP.

## See Also

- [Slack Integration](/integrations/slack/), publishing an agent to a Slack workspace
