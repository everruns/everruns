---
title: "Gemini citations survive every continuation hop"
description: "Long Gemini responses retain citations from each grounded request and count repeated searches across hops while preserving combined token usage."
date: 2026-10-19
kind: fix
significance: minor
products: [framework, platform]
release: "0.49.0"
surfaces: [api, code]
links:
  docs: [/providers/gemini/]
---

Long responses using Gemini's native Google Search now retain citations from
every grounded continuation request. Previously, a later grounded request could
replace earlier sources and supported answer spans. Citation indices now stay
with their own request, including when different hops repeat the same answer text.

Search usage counts a query again when another hop performs it, while repeated
metadata within one hop stays deduplicated. A final hop without grounding keeps
earlier citations. Token accounting and continuation limits are unchanged. See
[Google Gemini](/providers/gemini/) for long-decode configuration.
