---
title: Unfinished native web searches show failed status
description: OpenAI Responses searches that stop before completion now show as failed, while partial answers, citations, and provider usage remain available.
date: 2026-10-28
kind: fix
significance: minor
products: [platform, framework, cloud]
release: "0.50.1"
surfaces: [agent, code]
links:
  docs: [/capabilities/web-search/]
---

Native web searches on OpenAI Responses now report success only when the provider
explicitly marks the search completed. Searches stopped by tool-call limits or
other interruptions can end with a searching or in-progress status; these now
appear as failed hosted calls instead of successful searches.

Partial answer text and citations remain available, and the provider's usage
still contributes to cost accounting. Everruns does not automatically retry the
search. This also applies to OpenRouter search items on the Responses protocol.
No configuration change is required; see [Web search](/capabilities/web-search/).
