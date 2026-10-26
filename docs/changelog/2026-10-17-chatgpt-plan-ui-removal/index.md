---
title: ChatGPT Plan removed from Platform setup
description: The Platform UI no longer offers ChatGPT Plan sign-in or its models because its browser callback is unavailable to remote users.
date: 2026-10-17
kind: fix
significance: minor
products: [platform]
release: "0.49.0"
surfaces: [ui]
links:
  docs: [/features/chatgpt/]
---

![The Connect provider list with API providers available and no ChatGPT Plan sign-in](./provider-options@2x.png)

ChatGPT Plan no longer appears in the Platform provider setup, provider pages,
model lists, or chat controls. Its open-source sign-in flow requires a browser
callback on `127.0.0.1`, which cannot reach a remote Everruns server. Existing
connections remain stored, and the framework driver remains available to custom hosts.
See the [ChatGPT Plan framework guide](/features/chatgpt/) for supported usage.
