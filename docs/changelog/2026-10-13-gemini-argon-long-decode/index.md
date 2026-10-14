---
title: "Gemini Argon continues long responses across requests"
description: "Gemini 4 Argon can continue beyond a single stream, preserving the total output budget and combining text, reasoning, and billed usage across requests."
date: 2026-10-13
kind: feature
significance: notable
products: [framework, platform, cloud]
surfaces: [api, ui]
links:
  docs: [/providers/gemini/]
---

Everruns now supports long decode on Gemini 4 Argon. Raise the output budget
above 65,536 tokens to let one model response continue across multiple requests.
The default stays at 65,536; input, thinking, and output share Google's unified
context window.

In an agent package:

```toml
schema_version = 1
name = "deep-reasoning"
instructions = "Work through the problem carefully."
max_output_tokens = 100000

[model]
provider = "gemini"
model = "gemini-4-argon"
```

Text and reasoning stream as one response. Usage combines every completed hop,
including thinking and repeated input. Invalid continuation tokens or a failed
hop fail the call. Cancellation stops local continuation requests.

Argon also gains its built-in model profile and low, medium, and high thinking
levels. The server seeds it as an enabled favourite for Google Gemini, so it is
available in the model picker. New Gemini setups prefer Argon when choosing an
organisation default; an existing default stays selected. See
[Google Gemini](/providers/gemini/) for configuration, host timeouts, and recovery
limits.
