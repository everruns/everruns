---
title: "Remote workers run turns without an encryption key on the API"
description: "A standalone gRPC worker no longer fails every turn when the API has no SECRETS_ENCRYPTION_KEY; it skips checkpoints like the in-process worker."
date: 2026-10-13
kind: fix
significance: minor
products: [platform]
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
  prs: [everruns/everruns#4709]
---

A self-hosted API started without `SECRETS_ENCRYPTION_KEY` refused the encrypted
compaction checkpoint reads of a standalone gRPC worker, so every turn on that
worker failed with `processing_error`. The worker now asks the API at startup,
and again on a refused read, whether it can store checkpoints. Without a key it
runs turns without compaction checkpoints, native async tool journals or the
OpenAI Agents API runtime, the same way the in-process worker already did, and
logs a warning once. Set the key to keep those features; see
[Environment variables](/sre/environment-variables/).
