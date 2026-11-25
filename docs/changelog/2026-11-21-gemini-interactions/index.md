---
title: Gemini agents stop resending the whole conversation
description: Gemini now keeps the conversation on Google's side through the Interactions API, so each agent turn sends only the new tool results and messages.
date: 2026-11-21
kind: improvement
significance: notable
products: [framework, platform, cloud]
surfaces: [api, code]
links:
  docs: [/providers/gemini/]
---

A Gemini agent running twenty tools used to resend the whole conversation
twenty times: every message, result and thought signature.
[Gemini](/providers/gemini/) now uses Google's Interactions API, which stores
each turn on Google's side; the next call sends that turn's id and what is new:

```json
{
  "model": "gemini-4-hydrogen",
  "previous_interaction_id": "v1_ChdYdjBBYTc3",
  "input": [
    {
      "type": "function_result",
      "call_id": "call_7",
      "name": "get_weather",
      "result": "18C, light rain"
    }
  ]
}
```

OpenAI agents do this through the Responses API; Gemini agents
now do too. The system prompt and tools still go with each call. If Google
lost the stored turn, Everruns resends the transcript once.

Batch requests, explicit cached content, long decode past 65,536 output
tokens and custom base URLs stay on `generateContent`.

## Update 2026-11-23

The [Gemini Stateless](/capabilities/gemini-stateless/) capability opts out of
interaction storage and replays the full signed transcript on each call.

## Update 2026-11-25

A text-only tool result returns as a plain string, not a one-element content
array: Google treats that array as *multimodal*, which only Gemini 3+ accept.
Results carrying media keep the array.
