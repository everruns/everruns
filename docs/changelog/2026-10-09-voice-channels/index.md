---
title: "Voice channels let people call an agent"
description: "A voice channel lets people talk to an agent over WebRTC while it keeps its own model, tools and instructions, on the platform and in the Framework."
date: 2026-10-09
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.47.0"
links:
  docs: [/features/voice/, /framework/voice/]
  prs: [everruns/everruns#4394, everruns/everruns#4402, everruns/everruns#4406]
visual: "Agent Integrations tab with a voice channel and its talk button (/agents/<id>), cropped to the channel card"
---

Voice is now a channel type. A speech-to-speech model only listens and speaks;
the agent, with its own model and tools, writes every answer. Speech starts
sentence by sentence while the answer streams, short fillers cover silence, and
talking over the agent stops it. A call and a text chat on one session share a
transcript.

**In the UI:** an organization administrator turns on **Voice** in
**Settings → Features**. Then add a voice channel from the agent's
**Integrations** tab and use its talk button. Speech uses an OpenAI provider.

**In code:** `everruns::voice` puts a voice channel in front of any Framework
session, and serve's `voice` feature gives every top-level agent browser calls.

See [Voice](/features/voice/), [Framework voice](/framework/voice/) and [#4394](https://github.com/everruns/everruns/pull/4394), [#4402](https://github.com/everruns/everruns/pull/4402), [#4406](https://github.com/everruns/everruns/pull/4406).
