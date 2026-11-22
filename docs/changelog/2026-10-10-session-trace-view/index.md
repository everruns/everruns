---
title: "Session Trace view replaces Transcript and Timeline"
description: "Sessions open on a Trace tab that lays out turns and steps with timing bars, an inspector for each step's input, output and model request, and event search."
date: 2026-10-10
kind: feature
significance: major
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
release: "0.48.0"
links:
  docs: [/features/ui/#sessions, /reference/cli/sessions/]
  prs: [everruns/everruns#4467, everruns/everruns#4481, everruns/everruns#4500, everruns/everruns#4503, everruns/everruns#4504, everruns/everruns#4556]
---

![Session Trace tab with two turns, the step waterfall and the inspector open on a write_file call](./session-trace@2x.png)

**In the [UI](/features/ui/#sessions):** a session now opens on **Trace**, which replaces the Transcript
and Timeline tabs. A turn rail with a minimap leads to each turn's steps: model
calls, tool calls, approvals, sub-agents and messages, with waterfall timing
bars. The step inspector shows input, output, the full model request and raw
events. Batches and sub-agents expand inline, search jumps to a hit, and `j`
and `k` move between steps. The view, error filter and selected step live in
the URL.

**In code:** read the same data from `/v1/sessions/{id}/trace`, or with
[`everruns sessions trace get`](/reference/cli/sessions/) and `everruns sessions trace turns list`.

## Update 2026-11-20

Open a message or the tools list straight from the step inspector. The model
request opens in a near-full-screen modal with image previews and file downloads
for attachments retained in the trace. **View full message** opens the
original turn input in the sidebar.
