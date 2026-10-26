---
title: Dev mode PostgreSQL requires a non-root user
description: Development mode now uses the PostgreSQL lifecycle library and cleans up its temporary database when the owning server process exits.
date: 2026-10-13
kind: breaking
significance: notable
products: [platform]
release: "0.49.0"
surfaces: [api]
links:
  docs: [/sre/environment-variables/]
---

Development mode now requires a non-root user. Previously, starting it as root
switched the database process to `nobody`. Run the server under a regular account:

```bash
DEV_MODE=true ./everruns-server
```

Privileged environments can instead use external PostgreSQL with `DEV_MODE=false`.
Temporary clusters now stop and remove their private data even when the owning
process is forcibly terminated. Database queries, migrations, and isolated test
fixtures continue to use real PostgreSQL. See
[environment variables](/sre/environment-variables/#dev_mode) for setup details.
