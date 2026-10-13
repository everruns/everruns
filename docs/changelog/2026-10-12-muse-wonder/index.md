---
title: Muse Wonder, Meta's new top model, is available in Everruns
description: Meta's Muse Wonder joins the model catalog with pricing, a 1M-token context and its model card, and every Muse model gets a reasoning effort picker up to Max.
date: 2026-10-12
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.48.0"
links:
  docs: [/providers/meta/]
  prs: [everruns/everruns#4637, everruns/everruns#4645]
---

![Everruns and Meta logos, announcing the Muse Wonder model](./card@2x.png)

Meta shipped **Muse Wonder** (`muse-wonder-1.0`, preview) today, and you can put it to work in
Everruns right away. Wonder is built for the long jobs: extended agent runs, changes that span
many files, and orchestrating sub-agents. It reads 1,048,576 tokens of
context plus images, audio, video and PDFs. It costs $4.00 input, $0.20 cached and $20.00 output per
million tokens. Add a [Meta](/providers/meta/) provider, sync models, and Wonder shows up with a
link to Meta's model card.

Want it to think harder? Every Muse model now has a reasoning effort picker, Minimal through Max (Muse Spark 1.2 stops at
Extra High). Set it per message:

```bash
curl -X POST http://localhost:9300/api/v1/sessions/{session_id}/messages \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": {"role": "user", "content": [{"type": "text", "text": "Plan the move of our billing service to the new ledger"}]},
       "controls": {"reasoning": {"effort": "max"}}}'
```
