---
title: A2A callers sharing an endpoint now see only their own tasks
description: When an A2A endpoint verifies each caller with OIDC, OAuth2 introspection or mTLS, tasks belong to the caller who started them.
date: 2026-11-08
kind: fix
significance: minor
products: [platform]
release: "0.52.0"
surfaces: [api]
links:
  docs: [/features/a2a/]
---

An [A2A endpoint](/features/a2a/) secured with OIDC, Google sign-in, OAuth2 introspection or
mTLS can let many callers in, for example everyone at `acme.com`. Until now
they shared one task space: any of them could list, read, continue or cancel
another caller's tasks.

Tasks now belong to the verified caller who started them. `ListTasks` returns
only that caller's tasks, and `GetTask`, `CancelTask`, `SubscribeToTask`, push
configs and continuing a `taskId` or `contextId` answer "task not found" for
anyone else's. On a `shared_session` endpoint each caller gets their own
conversation instead of joining one shared session.

Endpoints secured with the generated API key or HTTP Basic have a single
credential, so they behave as before. Tasks created on a multi-caller endpoint
before this change are not tied to a caller, so callers no longer see them.
