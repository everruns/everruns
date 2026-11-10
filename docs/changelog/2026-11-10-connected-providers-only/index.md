---
title: "Models only lists the providers you connected"
description: "A fresh install no longer adds every built-in provider, keyless models stay disabled, and the default chat model follows the provider you actually connected."
date: 2026-11-10
kind: improvement
significance: minor
products: [platform]
surfaces: [ui]
---

A new Everruns no longer creates a card for every built-in provider. **Models →
Providers** starts empty apart from **Connect provider**, and a provider appears
once you connect it, or at startup when a `DEFAULT_*_API_KEY` is set for it.
Models of a provider without a key are never enabled, so the model picker only
offers models that can run.

Connected providers come first. Any left over from an older install sit under
**Not connected** as quiet cards with a **Connect** button, which also offers
OAuth sign-in for OpenRouter.

The default chat model now follows what you connected. Connect only OpenRouter
and chat defaults to an OpenRouter model instead of failing with "The AI
provider is not set up correctly". A provider connected from env discovers its
models right away instead of showing **Last synced: Never** until the hourly
sync.
