---
title: System Doctor
description: Check that a self-hosted Everruns install is set up right and working, and see what to fix when it is not
sidebar:
  order: 25
---

The system doctor checks every part of an install and reports each finding in
plain words: what it found, why it matters and how to fix it. It covers the
database and its migrations, connected workers (their load, versions, and what
each worker finds about its own setup, such as whether it reaches the egress
proxy), sign-in and
encryption keys, NATS, Valkey, S3 file storage and monitoring.

It works the same way however you host Everruns. It reads your configuration,
so a part you do not use (for example NATS on a single server) shows as a note
explaining what that means, not as a failure. On a production install
(`DEPLOYMENT_GRADE=prod`, the default) unsafe settings such as sign-in turned
off are problems; on a development install they are warnings.

## Run it

You need to be a platform user.

```bash
everruns system doctor --refresh true
```

The same report is at `GET /v1/system/doctor`. Without `refresh` you get the
report from the last minute; the checks write a small test file to S3 and
connect to NATS and Valkey, so they do not run on every request.

Each check has a `status`:

| Status | Meaning |
|---|---|
| `ok` | Working as intended. |
| `info` | Not used by this install, or worth knowing. Never a fault. |
| `warning` | Works, but degraded or risky. |
| `problem` | Broken or unsafe now. |

The report's own `status` is the worst of its checks, ignoring `info`, so a
script can fail on `problem`:

```bash
everruns system doctor --refresh true --output json | jq -e '.status != "problem"'
```

## Readiness probe

`GET /health/ready` answers `200` once the database is reachable and its
schema matches the server's version, and `503` otherwise. It needs no sign-in.
Use it as the readiness probe, and keep `/health` as the liveness probe so a
database outage does not get healthy servers restarted.
