---
title: "Gemini Decision 1 answers typed decisions"
description: "Google's new decision model joins Jev, GPT-6 Luna and Microsoft-Decision-1. Add it under a Gemini or OpenRouter provider for calibrated answers."
date: 2026-11-12
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/integrations/typesafe/, /framework/models-and-providers/]
---

[Gemini Decision 1](https://aistudio.google.com/docs/decisions-api), released by Google on
November 11, now answers Everruns decision calls. It returns calibrated probabilities for
yes/no, choice and score questions in one call (Google quotes 150 to 250 ms), at $0.10 per
million input tokens with free output.

**In the UI:** under **Registries → Models**, add a **Decisions** model with ID
`gemini-decision-1` to your Google Gemini provider, or `google/gemini-decision-1` to OpenRouter.
Pick it as the organization's decision default or in the **Jev Decisions** capability. The saved
Gemini key serves chat and decisions alike; see [TypeSafe](/integrations/typesafe/).

**In code:** the Gemini provider is a decisions service (feature `gemini`):

```rust
use everruns::{Decisions, Provider, providers::gemini::Gemini};

let gemini: Provider = Gemini::from_env()?.into();
let answers = Decisions::new("gemini-decision-1", gemini)
    .about("My card was charged twice and I need this fixed today.")
    .noul("urgent", "Does this convey urgency?")
    .choice("team", "Which team should handle this?", ["billing", "technical"])
    .send()
    .await?;
```

The driver table in [Models and providers](/framework/models-and-providers/) lists every
decision provider.
