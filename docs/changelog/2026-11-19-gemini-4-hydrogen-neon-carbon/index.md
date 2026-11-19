---
title: "Gemini 4 Hydrogen, Neon and Carbon join Argon"
description: "Google's three new Gemini 4 models have built-in profiles, pricing, thinking levels and long decode. New Gemini setups now default to Hydrogen."
date: 2026-11-19
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/providers/gemini/]
---

Google shipped three more Gemini 4 models this month, and the
[Google Gemini](/providers/gemini/) provider now knows them: **Gemini 4 Hydrogen** ($1.50 / $7.50 per million input / output tokens),
the fast everyday model Google suggests starting with; **Gemini 4 Neon**
($3 / $15), tuned for complex software engineering; and **Gemini 4 Carbon**
($2 / $10 until November 30, then $4 / $20), Google's strongest reasoner.

All four now share Argon's long decode: raise
`max_output_tokens` above 65,536 and one response continues across requests up
to the 1,048,576-token window. Hydrogen and Neon also take the new **Extra
High** thinking level; Carbon and Argon stop at High.

**In the UI:** a Gemini provider seeds Hydrogen, Neon and Carbon next to Argon.
A newly connected Gemini provider picks Hydrogen as the organization default,
Neon for coding and Carbon for research; existing defaults stay.

**In code:** use the model id with the `gemini` provider:

```rust
use everruns_contracts::{LlmCallConfig, ReasoningEffort};

let mut config = LlmCallConfig::new("gemini-4-neon");
config.reasoning_effort = Some(ReasoningEffort::Xhigh);
```
