---
title: "Five more providers: Mistral, Fireworks, Vercel, Cloudflare, MAI"
description: "Yolop's /setup and --provider now reach Mistral, Fireworks AI, Vercel AI Gateway, Cloudflare Workers AI, and Microsoft MAI, and Google runs on the native Gemini driver."
date: 2026-10-16
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.21.0"
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#860]
---

Yolop can now talk to five more providers. In `/setup` or with `--provider`, pick **Mistral**, **Fireworks AI**, **Vercel AI Gateway**, **Cloudflare Workers AI**, or **Microsoft MAI**, each with its own API key flow. Cloudflare also asks for the account ID and MAI for the deployment endpoint, stored alongside the key.

Google moved to the native Gemini driver under the same `GEMINI_API_KEY` or `GOOGLE_API_KEY`, so existing setups keep working with no changes.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models) and [#860](https://github.com/everruns/yolop/pull/860).
