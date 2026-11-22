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
      "result": [
        {
          "type": "text",
          "text": "18C, light rain"
        }
      ]
    }
  ]
}
```

OpenAI agents have worked this way through the Responses API; Gemini agents
now do too, with nothing to configure. The system prompt and tools still go
with every call, so editing an agent takes effect on its next step. If Google
no longer has the stored turn, Everruns sends the full transcript once and
carries on.

Batch requests, explicit cached content, long decode past 65,536 output
tokens and custom base URLs stay on `generateContent`.
