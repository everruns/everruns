---
title: Bashkit recognizes empty workspace directories
description: Bashkit shell commands recognize newly created empty workspace directories, so recursive copies preserve their source folders on remote workers.
date: 2026-10-27
cloud: 2026-10-28
kind: fix
significance: notable
products: [framework, platform, cloud]
release: "0.50.0"
surfaces: [agent, code]
links:
  docs: [/capabilities/bashkit-shell/]
---

[Bashkit Shell](/capabilities/bashkit-shell/) now recognizes empty workspace
directories through the remote worker's metadata API. Previously, creating a
directory could succeed while a subsequent recursive copy treated it as missing.
Copying several source folders then failed; copying one could flatten its
contents into the destination.

Normal shell installation steps preserve their directory layout:

```bash
mkdir -p /workspace/installed
cp -r /workspace/downloaded/skills/* /workspace/installed/
```

The same metadata checks serve embedded interpreters. Missing paths still report
as absent, and workspace access continues to follow the configured filesystem
mounts and permissions.
