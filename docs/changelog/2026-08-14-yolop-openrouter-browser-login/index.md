---
title: "OpenRouter connects with a browser sign-in"
description: "Yolop's /setup can sign in to OpenRouter in the browser with PKCE and store the minted API key, so you no longer paste or export a key by hand."
date: 2026-08-14
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.15.2"
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#567]
---

![Yolop and OpenRouter logos side by side, announcing the OpenRouter browser sign-in](./integration@2x.png)

OpenRouter can now be connected from the browser. In `/setup`, pick OpenRouter
and then **Sign in with browser**: Yolop opens `openrouter.ai/auth`, waits on a
loopback callback, exchanges the code for an API key and stores it the same way
as a pasted key. ACP clients see the method as `openrouter_browser`.

A pasted key and `OPENROUTER_API_KEY` still work, and the environment variable
wins when both are set.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models) and [#567](https://github.com/everruns/yolop/pull/567).
