---
title: Yolop preserves immediate ACP settings updates
description: Editors connected to Yolop receive model option updates when settings change immediately after opening or loading an ACP session, including later sessions.
date: 2026-10-31
kind: fix
significance: minor
products: [yolop]
surfaces: [cli]
links:
  prs: [everruns/yolop#898]
---

Yolop now preserves settings changes made immediately after an editor opens or
loads an ACP session. Previously, an edit before the settings watcher's first
poll could leave the editor's model options unchanged until another edit.

The watcher compares settings with the options already sent to each session.
This also covers new sessions joining an existing connection. Edit
`settings.toml` as usual; connected editors receive the updated model options
without reconnecting or making a second change.
