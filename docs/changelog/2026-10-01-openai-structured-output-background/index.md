---
title: OpenAI structured output and background mode for long reasoning
description: OpenAI drivers enforce JSON Schema structured output, and xhigh or max reasoning calls run as background responses that resume after a drop or worker restart.
date: 2026-10-01
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [api, code]
release: "0.34.0"
links:
  docs: [/providers/openai/]
  prs: [everruns/everruns#3932, everruns/everruns#3969]
---

A direct model call can require a reply that matches a JSON Schema. Everruns
sends it as `text.format` on the OpenAI Responses API and `response_format` on
Chat Completions. A driver that cannot enforce the schema refuses the call
instead of dropping it.

Calls at `xhigh` and `max` reasoning effort on OpenAI and Azure run as
background responses. If the stream drops or the worker restarts, Everruns
picks the response back up instead of paying for it twice. A cancelled turn
cancels the response, and the `openai/background` driver option overrides the
policy.

See [OpenAI](/providers/openai/). PRs [#3932](https://github.com/everruns/everruns/pull/3932), [#3969](https://github.com/everruns/everruns/pull/3969).
