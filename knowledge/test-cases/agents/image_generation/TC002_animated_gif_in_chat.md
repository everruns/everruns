---
type: Test Case
title: "TC002: Animated GIF on the Organization Default Image Model"
description: "Verify that a Worker-kit agent with no image settings draws an animated GIF on the organization's default image model, stores it as an image/gif artifact, and that chat plays it with a frame badge, pause and download."
tags:
  - everruns
  - test-case
  - agents
  - image-generation
---
# TC002: Animated GIF on the Organization Default Image Model

## Description

Verify that a Worker-kit agent with no image settings draws an animated GIF on the
organization's default image model, stores it as an `image/gif` artifact, and that chat
plays it with a frame badge, pause and download.

## Preconditions

- Control plane running
- An OpenAI provider with credit, with `gpt-image-2.6-flare` enabled
- **Models → Defaults → Images** set to GPT Image 2.6 Flare (or left on the platform default)
- `gpt-6.1-sol` enabled for chat

## Steps

1. Create an agent on the Worker harness with model `gpt-6.1-sol`. Do not add or configure
   Image Generation: the Worker kit carries it.
2. Open chat and send: `Make a short looping GIF of a paper lantern swaying at dusk.`
3. Wait for the turn to finish.
4. In the tool row, check the player badge, press Pause, then Play, then Download.
5. Fetch the artifact from `/v1/images/{artifact_id}` in the tool result JSON.

## Expected Result

- One `generate_image` call with `format: gif`, no `size` sent to the provider
- Tool result JSON: `format: gif`, one image with `media_type: image/gif`, `frames` > 1,
  an `img_*` `artifact_id`, and `model: gpt-image-2.6-flare`
- Chat shows the animation at full size with a `GIF · N frames · X s loop` badge, no
  thumbnail strip; Pause shows the first frame, Download saves `<prefix>.gif`
- `/v1/images/{id}` returns `image/gif` bytes starting with `GIF89a`
- The final reply describes what moves

## Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Size sent with GIF | Provider error `No supported sheet fits N GIF frames` |
| Wrong model | Result `model` is not the org default |
| Static preview only | Chat shows a PNG thumbnail instead of the animation |
| OpenRouter chosen for GIF | Error that the provider cannot make `gif`; GIF needs OpenAI or Azure OpenAI |
