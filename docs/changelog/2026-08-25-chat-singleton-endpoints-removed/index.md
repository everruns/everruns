---
title: Global chat session endpoints removed from the Everruns API
description: POST /v1/sessions/chat and POST /v1/sessions/chat/voice are gone; create a chat thread as an ordinary session with POST /v1/sessions instead.
date: 2026-08-25
kind: breaking
significance: minor
products: [platform, cloud]
surfaces: [api]
release: "0.22.0"
links:
  docs: [/built-ins/harnesses/platform-chat/]
  prs: [everruns/everruns#3278]
---

The two endpoints that looked up a per-user Platform Chat session by tag are removed. A chat thread is now an ordinary session created with `POST /v1/sessions` and bound to an agent.

After upgrading, `POST /v1/sessions/chat` returns `405` and `POST /v1/sessions/chat/voice` returns `404`. Sessions already tagged `global-chat` stay listed and readable through the normal session routes, and the Platform Chat harness is unchanged.

See [Platform Chat](/built-ins/harnesses/platform-chat/) and [#3278](https://github.com/everruns/everruns/pull/3278).
