---
type: Specification
title: System Doctor
description: One plain-language report on whether a whole Everruns install is set up right and working, adapted to every hosting mode.
tags:
  - everruns
  - operations
  - monitoring
---

# System Doctor

## Intent

An install is several moving parts: API servers, workers that connect back
over gRPC, PostgreSQL, optionally NATS, Valkey and S3, an egress proxy,
sandbox providers and monitoring. Each part had its own partial "is it OK"
signal (`/health`, `everruns docker doctor`, the Durable pages, Prometheus).
None answered the operator's question: **is this install set up right and
working, and if not, what exactly do I fix?**

The doctor answers it. Every check states what is true, why it matters and
how to fix it, in the same shape on every surface.

## Boundaries

- **Platform, not organization.** The report describes every server, worker
  and datastore, so it needs the platform-user grant (`system.view`).
  Organization problems (Slack scopes, the active-turn cap) stay in
  [Actionable Health Issues](health-issues.md); the two never share an inbox.
- **Deterministic.** No check runs an LLM generation or spends tokens.
- **No secrets.** Checks read presence and length. Connection strings are
  used by probes and never appear in a report.

## Adapting to the hosting mode

There is no hosting-mode setting and no feature flag. Each check decides from
configuration and live state whether it applies. One that does not apply
reports `info` with what that means ("NATS not used: live updates go through
PostgreSQL; fine for one server"), never a failure, and `info` never makes
the verdict worse. Strictness follows the
[deployment grade](../../crates/contracts/src/runtime/deployment.rs): an unsafe
setting is a `problem` on `prod` and a `warning` on `dev`.

| Mode | What changes |
|---|---|
| Local dev, `DEV_MODE` | In-process worker: worker authentication is a note. |
| Docker Compose | Profiles decide which of NATS, Valkey and metrics are checked. |
| Helm | Liveness stays on `/health`. Readiness moves to `/health/ready` once a released image serves it: the chart's CI installs the `latest` image, which a probe on a new path would fail. |
| SaaS | Adds its own checks through the server builder (later step). |
| serve, celld | Not covered: no control plane or workers. |

## Surfaces

- `GET /v1/system/doctor` (command `run_system_doctor`), and through it
  `everruns system doctor` and MCP. A report is cached for a minute; `refresh`
  runs the checks again, at most once every five seconds, because some checks
  touch S3, NATS and Valkey.
- `GET /health/ready`, unauthenticated: the database answers and its schema
  matches this build. A database migrated by a newer build still counts as
  ready, so the old replicas of a rolling deploy keep serving.

## Checks

The list lives in
[`domains/system/doctor/checks/`](../../crates/server/src/domains/system/doctor/checks),
one module per area: control plane (public address, utility model, server
replicas and their versions), database (reachable, version,
migrations, pools, long transactions), workers (connected, authentication,
workers turned away for a wrong token, versions and worker protocol, long
drains, capacity, waiting sessions, dead letters, circuit breakers, and the
workers' own reports), security (sign-in,
session signing secret, encryption key, stored secrets readable and rotation
finished), messaging (NATS, Valkey, both judged against the number of
running servers), file storage (an S3 write, read and
delete), observability (whether anything actually reads `/metrics`, traces,
log level) and platform model keys.

Two judgments worth keeping:

- "Monitored" means something reads `/metrics`, not that it is exposed. The
  server notes each scrape; exposed but unread for five minutes is a warning.
- Stored secrets under a key id the server lacks are a problem (those
  secrets are lost without that key); under a known older key only a warning
  (rotation unfinished). A sample is decrypted too, to catch a key whose id
  stayed while its value changed.

## Worker reports

Workers send their version, host name and
[worker protocol version](../../crates/internal-protocol/src/lib.rs) when they
register, and a short list of checks on their own setup at registration and
every five minutes
([`self_checks`](../../crates/worker/src/self_checks.rs)): whether the egress
proxy is reachable from the worker, whether sandbox egress settings are
usable, and whether the worker uses NATS. Only what differs per worker is
checked there; the server judges everything else. The server merges the
reports per check across workers and names the workers that differ. It
compares the NATS report with its own setting, because a worker with
`NATS_URL` against a server without it sends streamed output in a way only
NATS makes safe. Reports older than fifteen minutes are ignored.

Refused worker calls are counted per server process, so a worker locked out
by a wrong token shows up without reading the worker's log.

## Server replicas

Every server process writes a heartbeat to `system_instances` at start and
every 30 seconds ([`instances`](../../crates/server/src/domains/system/doctor/instances.rs)):
its version, newest migration, host name, and whether NATS and Valkey are
set. A doctor run on any replica reads them all. That gives the replica
checks: how many servers run and which stopped in the last hour, servers on
different versions (a warning during a rollout, a problem after ten
minutes), NATS and Valkey unset while several servers run (each polls the
database; rate limits multiply), and servers that disagree on either.

Seen within 90 seconds counts as running. Rows are kept after a process
stops, so a restarted pod is a new row and a stopped one stays as history.
The table has a `kind` column so other process types can join it; the egress
proxy has no database access in the open-source build, so its replicas are
covered where its control endpoint lives (the SaaS checks).

## Planned

The System → Health page, chaining
`everruns docker doctor` into it, an `everruns-server doctor` preflight, a
Prometheus gauge per check, SaaS checks and the release gate.
