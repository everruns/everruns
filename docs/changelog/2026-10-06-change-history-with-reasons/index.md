---
title: "Change history with reasons and restore for every entity"
description: "Every configuration change records who made it, through which surface and why, with numbered revisions you can compare and restore from the History menu."
date: 2026-10-06
kind: feature
significance: major
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
release: "0.42.0"
links:
  docs: [/features/change-history/, /reference/cli/history/]
  prs: [everruns/everruns#4228, everruns/everruns#4264, everruns/everruns#4260, everruns/everruns#4271, everruns/everruns#4272]
visual: "Agent page with the History sheet open showing entries with reasons and a diff (/agents/<id>?sheet=history), cropped to the sheet"
---

Every mutating command now records one history entry: the entity, the action,
the changed fields, the actor, the surface, the session and an optional reason.
Configuration changes also create numbered revisions you can compare and
restore. Secrets appear only as changed or unchanged.

**In the UI:** open the **⋯** menu on an entity's page. **History** lists
changes with their reasons, and **Restore this point** asks for a reason.
**Manager notes** holds context agents read before changing the entity.

**In code:** pass a reason with `--reason` on the CLI or the
`Everruns-Change-Reason` header, and read history with
`everruns history list`, `show`, `diff` and `restore`.

See [Change history](/features/change-history/), the
[history CLI reference](/reference/cli/history/) and [#4228](https://github.com/everruns/everruns/pull/4228), [#4264](https://github.com/everruns/everruns/pull/4264), [#4272](https://github.com/everruns/everruns/pull/4272).
