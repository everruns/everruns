---
title: Feature flag grade preview renamed to internal for operators
description: Self-hosted operators must replace FEATURE_*=preview with internal, as rollout grades now set availability, organization defaults and who may enable a feature.
date: 2026-10-03
kind: breaking
significance: notable
products: [platform]
surfaces: [operations]
release: "0.38.0"
links:
  docs: [/sre/environment-variables/]
  prs: [everruns/everruns#4115, everruns/everruns#4137]
---

Each `FEATURE_<NAME>` variable now takes a rollout grade: `dev`,
`internal`, `adoption`, `prod` or `off`. `internal` features stay off
until a platform operator enables them for an organization, `adoption`
features are opt-in for organization owners and admins, and `prod` features
are on by default.

Upgrade your configuration:

- `FEATURE_*=preview` becomes `internal`.
- `true` becomes `adoption` to keep organization opt-in, and `false` becomes
  `off`.

Invalid values, including the old ones, disable the feature. The separate
`DEPLOYMENT_GRADE=preview` is unchanged.

See [Environment variables](/sre/environment-variables/). PRs
[#4115](https://github.com/everruns/everruns/pull/4115), [#4137](https://github.com/everruns/everruns/pull/4137).
