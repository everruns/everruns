---
title: "Rate agent replies and branch a chat from any message"
description: "Agent replies in Chat get Good, Bad and Branch into new chat actions, and each message shows one status row from sending until the answer arrives."
date: 2026-10-10
kind: improvement
significance: notable
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
links:
  docs: [/features/ui/#chat-and-threads]
  prs: [everruns/everruns#4502, everruns/everruns#4494, everruns/everruns#4480]
visual: "Chat with an agent reply showing Good, Bad and Branch into new chat actions and a Worked for status row (/chat), cropped to two messages"
---

**In the UI:** each agent reply in Chat has **Good** and **Bad** ratings, kept
per person, and **Branch into new chat**, which copies the conversation up to
the end of that reply's turn. Every message you send shows one status row
(Sending, Queued, Thinking, Working, Worked for) and **Retry** if it was not
delivered.

**In code:** `POST /v1/sessions/{id}/messages` accepts an optional
`client_message_id`. Sending the same one again returns the stored message
instead of starting a second turn, and the response reports whether the message
started, steered or resumed a turn. The CLI takes `--client-message-id`.

See [Chat and threads](/features/ui/#chat-and-threads) and [#4502](https://github.com/everruns/everruns/pull/4502), [#4494](https://github.com/everruns/everruns/pull/4494), [#4480](https://github.com/everruns/everruns/pull/4480).
