---
title: Organization default model can fall back to the platform default
description: An organization's default model is now an explicit override that admins can clear, falling back to a platform default shown in Models and Settings.
date: 2026-08-31
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
release: "0.24.0"
links:
  docs: [/reference/cli/models/]
  prs: [everruns/everruns#3114, everruns/everruns#3321]
---

A new organization no longer stores a concrete default model. When none is set, the platform default applies, and the Models and Organization settings pages label that state **Platform default** with the model name.

To go back to the platform default after choosing a model, select the platform default option, or send `"default_model_id": null` in `PATCH /v1/orgs/{org}`. Omitting the field leaves the setting unchanged.

See [CLI: models](/reference/cli/models/), [#3114](https://github.com/everruns/everruns/pull/3114) and [#3321](https://github.com/everruns/everruns/pull/3321).
