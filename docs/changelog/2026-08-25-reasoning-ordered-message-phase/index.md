---
title: Messages API reports reasoning in order and the message phase
description: Session messages now carry ordered reasoning parts plus phase and phase_source, so clients can tell intermediate commentary from the final answer.
date: 2026-08-25
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [api, code]
release: "0.22.0"
links:
  docs: [/event-reference/]
  prs: [everruns/everruns#3276, everruns/everruns#3279]
---

Model reasoning is now stored as ordered parts of a message instead of two flat strings. `GET /v1/sessions/{id}/messages` returns reasoning as ordered `reasoning` content parts, with provider replay state removed, and adds `phase` (`commentary` or `final_answer`) and `phase_source` (`provider` or `derived`).

Replay to providers is also more accurate: Anthropic thinking blocks keep their own signatures, including `redacted_thinking`; OpenAI reasoning items keep the ids the provider issued; and the Gemini driver now requests, parses and replays thoughts.

See [Event reference](/event-reference/), [#3276](https://github.com/everruns/everruns/pull/3276) and [#3279](https://github.com/everruns/everruns/pull/3279).
