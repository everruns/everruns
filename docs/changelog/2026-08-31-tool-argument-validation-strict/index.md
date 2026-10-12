---
title: Tool calls are validated against their schema before running
description: Everruns checks every tool call's arguments against the tool's JSON Schema and enables strict schemas for compatible OpenAI Chat and Responses tools.
date: 2026-08-31
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [agent]
release: "0.24.0"
links:
  docs: [/capabilities/tool-call-repair/]
  prs: [everruns/everruns#3336]
---

Malformed tool calls no longer reach tool implementations. Before a tool runs, its arguments are validated against the tool's registered JSON Schema. Missing, wrong-type or unknown arguments return a structured error the model sees, so it can correct the call on the next turn. Tool call repair still runs first.

For OpenAI Chat Completions and Open Responses, compatible tools are now sent with strict schemas. A schema that cannot be converted safely is sent non-strict, as before.

See [Tool Call Repair](/capabilities/tool-call-repair/) and [#3336](https://github.com/everruns/everruns/pull/3336).
