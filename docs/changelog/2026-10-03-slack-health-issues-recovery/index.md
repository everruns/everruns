---
title: Slack permission issues appear in Health with recovery steps
description: Slack installations missing a permission or credential raise a persistent issue in Settings > Health, with guided recovery, per-user snoozing and rechecks.
date: 2026-10-03
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, operations]
release: "0.39.0"
visual: "Settings > Health with a pending Slack permission issue and its recovery guidance; crop to the issue card."
links:
  docs: [/how-to/publish-to-slack/, /reference/cli/health-issues/]
  prs: [everruns/everruns#4134]
---

Adding scopes to new Slack app manifests does not repair bots that are already
installed. Everruns now checks existing Slack installations and records one
issue for each missing permission or credential problem.

Review issues in **Settings > Health**, beside the channel's settings, and
under **Action required** in the notification menu. Managed apps get a repair
that extends the app's manifest before Slack asks for consent again; manually
configured apps get scope, reinstall and token guidance. Reading or snoozing an
issue keeps it active until a fresh check passes. From the CLI, use
`everruns health-issues`.

See [Resolve installation issues](/how-to/publish-to-slack/#resolve-installation-issues).
PR [#4134](https://github.com/everruns/everruns/pull/4134).
