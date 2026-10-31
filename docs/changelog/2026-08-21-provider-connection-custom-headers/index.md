---
title: Provider connections carry custom headers and cache diagnostics
description: An organization can set extra HTTP headers and a prompt cache diagnostics opt-in on a provider connection, applied to every model call made through it.
date: 2026-08-21
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.21.0"
links:
  docs: [/providers/]
  prs: [everruns/everruns#3232, everruns/everruns#3240]
---

A provider connection now has request options that apply to every call through it, such as a header a gateway requires.

![Provider Advanced card with two custom headers and Prompt cache diagnostics turned on](./provider-advanced-headers@2x.png)

**In the UI:** open a provider in **Settings → Providers** and use the **Advanced** card to add **Custom headers** or turn on **Prompt cache diagnostics**.

**In the API:** the provider settings accept the same options. Transport headers such as `host` and `content-length` are rejected, the list is limited to 16 headers of up to 2 KB each, and header values are redacted in diagnostics. The Anthropic, Gemini, Chat Completions and Open Responses drivers apply them.

See [Providers](/providers/), [#3232](https://github.com/everruns/everruns/pull/3232) and [#3240](https://github.com/everruns/everruns/pull/3240).
