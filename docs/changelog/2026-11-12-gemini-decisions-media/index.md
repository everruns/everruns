---
title: "Gemini decisions read images, audio, video and PDFs"
description: "Attach session files to a Jev decision on Gemini Decision 1, and let a deployment answer its guardrail checks with Gemini through one environment variable."
date: 2026-11-12
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [agent, code, operations]
links:
  docs: [/integrations/typesafe/]
---

An agent can now ask [Gemini Decision 1](https://aistudio.google.com/docs/decisions-api) about
a photo, a call recording, a video or a PDF, not just text. The `jev_decision` tool takes
session file paths in `files`, and every question is answered over the text and the files
together:

```json
{
  "state": "Delivery photo for order 1042.",
  "files": [
    "/workspace/delivery.jpg"
  ],
  "questions": [
    {
      "id": "damaged",
      "type": "noul",
      "instructions": "Is the parcel visibly damaged?"
    }
  ]
}
```

**In code:** attach media to any request with `DecisionRequest::attach(DecisionMedia::bytes(...))`.
Models that read text only refuse such a request instead of answering without the file.

**In operations:** `UTILITY_DECISION_DRIVER=gemini` with `UTILITY_GEMINI_API_KEY` answers the
deployment's guardrail checks with Gemini Decision 1. See [TypeSafe](/integrations/typesafe/).
