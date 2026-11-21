---
title: "Mistral AI connects without a flag; machine payments and Docker Container leave"
description: "Mistral AI is in the provider picker for every organization. The never-enabled machine payments feature and the experimental Docker Container capability are removed."
date: 2026-11-21
kind: breaking
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/providers/mistral/, /capabilities/container-sandbox/]
---

[Mistral AI](/providers/mistral/) now sits in **Settings > Providers** next to every
other provider. Paste a La Plateforme key and Everruns syncs the models your plan
allows, Ministral and Codestral included. Chat Completions and the Responses API
both answer through the same driver. `FEATURE_MISTRAL` no longer does anything.

Two features that no deployment ever switched on are gone:

- **Machine payments.** The wallet, spend policy and attempt APIs, the
  `everruns payments` commands, **Settings > Payments** and the paid `parallel`
  capability are removed. The free `parallel_search` capability is unchanged.
  Any rows already stored stay in the database, encrypted.
- **Docker Container** (`docker_container`). Use the
  [Container sandbox](/capabilities/container-sandbox/) for self-hosted Docker
  execution, or a hosted sandbox provider.

`FEATURE_MACHINE_PAYMENTS` and `FEATURE_DOCKER_CAPABILITY` no longer affect
anything; remove them from your environment when convenient.
