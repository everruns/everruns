---
title: "Fix ChatGPT plan sessions failing with HTTP 400"
description: "Sessions that use a ChatGPT plan connection no longer fail with HTTP 400, because Everruns stops sending request-body metadata the Codex backend rejects."
date: 2026-10-09
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [api]
release: "0.47.0"
links:
  docs: [/features/chatgpt/]
  prs: [everruns/everruns#4464]
---

Sessions running on a ChatGPT plan connection could fail every model call with
HTTP 400, because requests to the Codex backend carried metadata in the request
body that it does not accept.

Everruns now leaves that metadata out of the request body while keeping the
session HTTP header and host observability metadata. No configuration change is
needed; existing ChatGPT connections work after upgrading.

See [ChatGPT plan](/features/chatgpt/) and [#4464](https://github.com/everruns/everruns/pull/4464).
