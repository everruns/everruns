---
title: Conversation APIs return reviewed message metadata
description: Internal replay and execution metadata stays in storage while public message annotations remain available.
date: 2026-10-28
kind: breaking
significance: notable
products: [platform, cloud, framework]
surfaces: [api, code]
links:
  docs: [/features/events/#public-metadata-and-replay-state]
---

Conversation APIs now return an explicit set of public message annotations.
Internal replay records and script execution inputs are withheld from message
responses, events, streaming, trace previews and JSONL exports, including reads
of previously stored messages. Stored history remains intact for recovery.

Applications returning arbitrary annotations should write them under
`metadata.custom`. Unknown existing keys remain stored but are no longer returned.
Client message IDs, model details, user-facing errors, initiator identity and page
context continue to work. Full prompt and tool diagnostics keep their existing
visibility contract. See [the metadata migration notes](/features/events/#public-metadata-and-replay-state).
