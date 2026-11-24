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
---

Voice is a channel for WebRTC calls. The agent keeps its model, tools and
instructions; speech streams sentence by sentence, and interruptions stop
playback. Calls and text chats share a transcript.

![Voice channel on the agent Integrations tab with its Talk to this channel button](./voice-channel-talk@2x.png)

**In the UI:** an administrator enables **Voice** in **Settings → Features**,
then adds a channel from the agent's **Integrations** tab.

**In code:** `everruns::voice` adds calls to Framework sessions; serve's `voice`
feature exposes browser calls for top-level agents.

Cascaded OpenAI speech defaults to `gpt-realtime-2.1-mini` over Realtime,
preserving 24 kHz PCM. Explicit Speech API overrides retain their transport
until [retirement on January 6, 2027](https://developers.openai.com/api/docs/guides/text-to-speech).
Exact-text instructions express intent; generative speech is not guaranteed
verbatim.

See [Voice](/features/voice/), [Framework voice](/framework/voice/) and
[#4394](https://github.com/everruns/everruns/pull/4394),
[#4402](https://github.com/everruns/everruns/pull/4402),
[#4406](https://github.com/everruns/everruns/pull/4406).
