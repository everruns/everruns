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
proxy), every running API server and whether they all run the same version,
sign-in and encryption keys, NATS, Valkey, S3 file storage and monitoring.

It works the same way however you host Everruns. It reads your configuration,
so a part you do not use (for example NATS on a single server) shows as a note
explaining what that means, not as a failure. On a production install
(`DEPLOYMENT_GRADE=prod`, the default) unsafe settings such as sign-in turned
off are problems; on a development install they are warnings.

Each API server records itself every 30 seconds, so the report is the same
whichever server answers it. Once a second server runs, NATS and Valkey stop
being optional notes: without NATS each server polls the database for the
others' updates, and without Valkey each server allows the full rate limit on
its own. Servers on different versions are a warning while a deploy rolls out
and a problem after ten minutes.

## Run it

In the app, open **System → Health**. It shows the verdict, what needs
attention with how to fix it, and every other check by area. **Check again**
runs the checks now; **Copy report** copies a plain-text report to paste into
an issue. The page and the API need a platform user.

From the CLI:

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

## Without signing in

The server binary runs the doctor on its own, against the services its
environment names, and exits `1` when a check is a problem (warnings exit
`0`). It needs no running server and no sign-in, so it works as a check before
a deploy and from inside a container:

```bash
everruns-server --doctor           # plain text
everruns-server --doctor --json    # the API's report shape
docker compose exec server /app/everruns-server --doctor
```

`everruns docker doctor` runs it for you when your local instance is up.

## Alerting

Every server exports each check as a Prometheus gauge,
`everruns_doctor_check_status{area,check}`, refreshed every minute: `0` ok or
not used, `1` warning, `2` problem. `everruns_doctor_status` is the whole
report on the same scale. Every replica exports the same checks, so alert on
the cluster's worst value:

```text
max without(instance) (everruns_doctor_check_status) == 2
```

## Readiness probe

`GET /health/ready` answers `200` once the database is reachable and its
schema matches the server's version, and `503` otherwise. It needs no sign-in.
Use it as the readiness probe, and keep `/health` as the liveness probe so a
database outage does not get healthy servers restarted.
