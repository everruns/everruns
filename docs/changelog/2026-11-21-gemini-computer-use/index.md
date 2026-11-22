---
title: Gemini 4 drives the screen with its own computer use tool
description: Gemini 4 agents now operate desktops and browsers through Google's built-in computer use tool, the one the model is trained on, not a described function.
date: 2026-11-21
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/computer-use/]
---

[Computer use](/capabilities/computer-use/) already swapped in the native tool
for OpenAI and Claude models. Gemini 4 agents now get Google's built-in
`computer_use` tool too, on a desktop sandbox or a Browserless browser. Gemini
answers with its own actions on a 0 to 999 grid:

```json
{
  "type": "function_call",
  "name": "click",
  "arguments": {
    "x": 500,
    "y": 500,
    "intent": "Click the Sign in button."
  }
}
```

Everruns turns that into a click at the real pixel on a 1280 by 800 display,
`[640, 400]`, and runs it through the same `computer` tool, approvals and
action budget as every other model. A safety flag Gemini puts on an action
stays on the call, and once the action has run Gemini is told it went ahead.

It needs nothing new: keep `native_tools` on (the default) and pick a Gemini 4
model on Google's own endpoint.
