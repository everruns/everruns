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

A Gemini agent that ran twenty tools in a row used to send the whole
conversation twenty times: every message, every tool result and every thought
signature, on every call. [Gemini](/providers/gemini/) now runs on Google's
Interactions API, which stores each turn on Google's side. The next call sends
the stored turn's id and only what is new since the model answered:

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

OpenAI agents have worked this way through the Responses API; Gemini agents
now do too, with nothing to configure. The system prompt and tools still go with every call. If Google
no longer has the stored turn, Everruns sends the full transcript once and
carries on.

Batch requests, explicit cached content, long decode past 65,536 output
tokens and custom base URLs stay on `generateContent`.

## Update 2026-11-23

The [Gemini Stateless](/capabilities/gemini-stateless/) capability opts out of
interaction storage and replays the full signed transcript on each call.

## Update 2026-11-25

A tool result carrying only text now goes back as a plain string instead of a
one-element content array. Google documents the array form as a *multimodal*
function response, which only Gemini 3 and later models accept, so Gemini 2.5
rejected the follow-up turn with a `400 INVALID_ARGUMENT` error. Results that
carry an image, document or screenshot keep the array.
