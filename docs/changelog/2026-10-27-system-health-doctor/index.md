---
title: System health doctor checks the whole install
description: One report on whether the database, workers, servers, keys, NATS, Valkey, S3 and monitoring are set up right, with what to fix when they are not.
date: 2026-10-27
kind: feature
significance: major
products: [platform]
surfaces: [ui, cli, api, operations]
links:
  docs: [/sre/system-doctor/]
  prs: [everruns/everruns#4963, everruns/everruns#4965, everruns/everruns#4968, everruns/everruns#4971, everruns/everruns#4973, everruns/everruns#4976]
---

**System → Health** checks every part of a self-hosted install and says, in plain words, what
it found, why it matters and how to fix it. It covers database migrations and pools, workers and
their versions, every API server, sign-in and encryption keys, NATS, Valkey, S3 file storage and
whether anything reads `/metrics`. A part you do not use shows as a note, so a laptop install and
a multi-replica cluster both come out clean when they are set up right.

The same report runs from a terminal with no sign-in, and exits 1 on a problem:

```bash
everruns-server --doctor          # inside the server container
everruns docker doctor            # a local Docker install
everruns system doctor --refresh true --output json
```

Each check is also the `everruns_doctor_check_status` gauge for alerting. See
[System doctor](/sre/system-doctor/).
