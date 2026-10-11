---
title: "Models and providers move into one registry page"
description: "Registries \u2192 Models now holds Models, Providers and Defaults tabs, so connecting a provider, choosing its models and setting org defaults happen in one place."
date: 2026-10-09
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, operations]
release: "0.47.0"
links:
  docs: [/features/ui/#models, /providers/]
  prs: [everruns/everruns#4458]
---

![Models page with the Connect provider sheet listing providers and how many of each are connected](./connect-provider@2x.png)

Providers moved out of Settings into **Registries → Models**, which has three
tabs. **Models** lists every model from every connected provider and flags new
ones a sync discovered for review. **Providers** connects a provider in one
sheet: driver, name and key, then the models to enable, with recommended ones
ticked. **Defaults** sets the organization's default model, decision model and
default providers per service.

Provider names are now unique within an organization. Managing providers and
models needs the `org:models:manage` permission, which replaces the separate
provider permission.

See [Models](/features/ui/#models), [Providers](/providers/) and [#4458](https://github.com/everruns/everruns/pull/4458).
