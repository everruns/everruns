---
title: Meta Muse sessions continue after tool calls in Yolop
description: Yolop keeps Meta Muse sessions running after tool calls by replaying context when a response ID cannot be used for server-side continuation.
date: 2026-10-16
kind: fix
significance: minor
products: [yolop]
surfaces: [cli, agent]
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#861]
---

Some Meta Muse responses carry IDs that Meta will not accept as a
`previous_response_id`. A Yolop session could finish a tool call, then stop
with HTTP 400 when it tried to continue from that ID.

Yolop now replays the saved conversation for those responses. Responses with
IDs that Meta accepts continue using server-side conversation state. The
session history and selected model stay the same.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models)
for Meta setup and model selection.
