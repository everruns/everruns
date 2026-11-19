---
title: Workers can save larger generated media files
description: Generated images and render exports can reach Session Files even when their encoded payload exceeds the ordinary worker command limit.
date: 2026-11-19
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/file-system/]
---

Workers can save larger generated images and render exports to Session Files.
The larger request allowance applies to session file writes. File quotas and
permissions still apply. See [File System](/capabilities/file-system/).

This helps agents keep the original output available for download and reuse in
later turns when the renderer or image provider produces a larger file.
