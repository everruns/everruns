---
title: Muse Wonder, Meta's new top model, is available in Everruns
description: Meta's Muse Wonder joins the model catalog with pricing, a 1M-token context and its model card, and every Muse model gets a reasoning effort picker up to Max.
date: 2026-10-12
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/providers/meta/]
  prs: [everruns/everruns#4637, everruns/everruns#4645]
---

![Everruns and Meta logos, announcing the Muse Wonder model](./card@2x.png)

**Muse Wonder** (`muse-wonder-1.0`, preview) is Meta's model for long agent runs, multi-file
coding and sub-agent orchestration. Add a [Meta](/providers/meta/) provider and sync models: Wonder
arrives with a 1,048,576-token context, text, image, audio, video and PDF input, and pricing of
$4.00 input, $0.20 cached input and $20.00 output per million tokens. Its row on the Models page
links Meta's model card.

Every Muse model now has a reasoning effort picker, Minimal through Max (Muse Spark 1.2 stops at
Extra High). Set it per message:

```bash
curl -X POST http://localhost:9300/api/v1/sessions/{session_id}/messages \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": {"role": "user", "content": [{"type": "text", "text": "Hello!"}]},
       "controls": {"reasoning": {"effort": "max"}}}'
```
