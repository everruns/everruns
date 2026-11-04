---
title: Automatic session titles
description: Automatically name conversations and update titles when the primary topic changes, independently of the agent's tools.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `automatic_session_titles` |
| **Category** | Session |
| **Tools** | None |
| **Dependencies** | None |

Enable **Automatic session titles** on a harness, agent, or session. The platform
checks the conversation after a completed turn without waiting on the title
model before delivering the agent's answer. The `session` capability and its
`write_session_title` tool are optional.

## Model selection

By default, titles use the organization's **Small model**, configured under
**Models → Defaults**. If that setting is empty, titles inherit the session's
model, including its agent, harness, and organization defaults.

In capability settings, choose **Use session model** to always inherit the
session model, or select a specific model and provider. An unavailable selected
model skips generation; it never switches accounts or uses deployment utility
credentials. Title requests count toward session usage and budgets.

```json
{
  "ref": "automatic_session_titles",
  "config": {
    "model_id": "session",
    "retitle": true
  }
}
```

Omit `model_id` to use the organization Small model default. Set `retitle` to
`false` to generate only the initial title.

## When titles change

The first check follows the first completed turn with a user message. Greetings
or insufficient context can leave the session untitled. Later checks require
at least five minutes and new user messages: 4 after a changed title, then 8,
16, and 32 between checks if the current title stays useful. The interval stays
at 32 until a title changes. Failed attempts also back off. Autonomous turn
continuations do not count as new user messages.

One bounded model request decides whether to keep the title or replace it.
There is no separate intent-classification request on every turn. The title
model sees a limited recent text transcript and no tool results or attachments.
Generation is best effort: an outage leaves the existing title in place.

## Explicit titles take priority

A title supplied at creation, renamed through the API or UI, or written with
`write_session_title` becomes explicit and is never overwritten automatically.
Writing the same text explicitly also takes ownership. An explicit rename wins
even when a title request is already running.

The existing [Session capability](/capabilities/session/) remains available for
agent-directed naming. Leave its `auto_title` flag off when using this
capability; enabling that flag asks the main agent to name the session explicitly,
which takes priority over automatic maintenance. Title text changes emit the
usual `session.title.updated` event.
