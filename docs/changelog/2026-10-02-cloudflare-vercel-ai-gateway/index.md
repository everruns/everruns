---
title: Cloudflare AI Gateway and Vercel AI Gateway model providers
description: Agents can run models through Cloudflare AI Gateway or Vercel AI Gateway, and the Cloudflare provider lists Workers AI text-generation models automatically.
date: 2026-10-02
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.34.2"
links:
  docs: [/providers/cloudflare/, /providers/vercel/]
  prs: [everruns/everruns#3996, everruns/everruns#4021]
---

![Everruns, Cloudflare and Vercel logos side by side, announcing the two AI Gateway providers](./integration@2x.png)

Two gateway providers are available. **Cloudflare** reaches many upstream
providers and Cloudflare's own Workers AI models through one account token and
account ID. **Vercel AI Gateway** uses its OpenAI-compatible Responses endpoint
and syncs its model list.

The Cloudflare provider lists Workers AI text-generation models with their
context window and tool support. Third-party model IDs on Cloudflare are still
added by hand, because no endpoint lists what an account can bill.

This also fixes a token dropped from Cloudflare streams when the model emitted
a purely numeric token.

See [Cloudflare](/providers/cloudflare/) and [Vercel](/providers/vercel/). PRs
[#3996](https://github.com/everruns/everruns/pull/3996), [#4021](https://github.com/everruns/everruns/pull/4021).
