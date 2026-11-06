---
title: "Gemini usage includes hosted-tool prompt tokens"
description: "Gemini token usage now includes provider-reported tool-use prompts, preserving cache accounting across streaming, continuation, recovery and batch responses."
date: 2026-11-04
release: "0.51.0"
kind: fix
significance: minor
products: [framework, platform]
surfaces: [api, code]
links:
  docs: [/providers/gemini/]
---

Gemini responses now count provider-reported tool-use prompt tokens as additional
uncached input. Previously, those tokens were omitted from reported input and
total usage. For example, 100 prompt tokens, 30 cached tokens and 7 tool-use prompt
tokens now report 77 uncached input tokens and 30 cached tokens.

The correction applies to streaming, whole responses and batch results.
Continuation requests retain their per-request billing totals; repeated cumulative
frames and checkpoint recovery do not count the same tool-use tokens twice. See
[Google Gemini](/providers/gemini/) for usage and continuation behavior.
