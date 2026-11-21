---
title: Fast mode for Claude Opus 5.5 and Fable 5.2
description: Turns on Claude Opus 5.5, Fable 5.2 and Mythos 5.2 can ask for Anthropic's fast mode with the same speed control OpenAI models use, priced at 2x Standard.
date: 2026-11-21
kind: feature
significance: minor
products: [platform, cloud, framework]
surfaces: [api]
links:
  docs: [/providers/anthropic/]
---

Claude Opus 5.5, Claude Fable 5.2 and Claude Mythos 5.2 now take the speed
control that GPT-6 models already use. Set `controls.speed` to `fast` (or
`priority`, the same tier) and the turn runs on Anthropic's fast mode: the same
model with faster output, at twice the Standard price.

```bash
curl -X POST http://localhost:9300/api/v1/sessions/{session_id}/messages \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"message": {"role": "user", "content": [{"type": "text", "text": "Hello!"}]},
       "controls": {"speed": "fast"}}'
```

Usage is priced at the speed Anthropic reports serving. Other Claude models,
Bedrock and gateways drop the setting, and batch requests run at Standard. See
[Anthropic](/providers/anthropic/#fast-mode).
