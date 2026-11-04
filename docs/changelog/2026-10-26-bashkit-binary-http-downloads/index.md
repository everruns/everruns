---
title: Bashkit curl preserves binary HTTP downloads
description: Bashkit curl now saves downloaded archives and other binary responses without altering their bytes, so agents can extract them in their workspace.
date: 2026-10-26
cloud: 2026-10-26
kind: fix
significance: notable
products: [bashkit, cloud]
surfaces: [agent, code]
links:
  docs: [/capabilities/bashkit-shell/]
  prs: [everruns/bashkit#2778]
---

Binary HTTP downloads through Bashkit's `curl -o` and `curl -O` now retain the
original response bytes. Gzip archives and other binary files no longer become
corrupted when saved into the virtual filesystem.

With outbound HTTP enabled, an agent can download an archive and extract it
directly:

```bash
curl -L -o skills.tar.gz https://github.com/cloudflare/skills/archive/refs/heads/main.tar.gz
tar -xzf skills.tar.gz
```

[Bashkit Shell](/capabilities/bashkit-shell/) continues to enforce the agent's
configured network access rules for these downloads.
