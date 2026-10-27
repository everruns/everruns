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
| Helm | `/health/ready` is the readiness probe; liveness stays on `/health`. |
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
one module per area: control plane, database (reachable, version,
migrations, pools, long transactions), workers (connected, authentication,
capacity, waiting sessions, dead letters, circuit breakers), security (sign-in,
session signing secret, encryption key, stored secrets readable and rotation
finished), messaging (NATS, Valkey), file storage (an S3 write, read and
delete), observability (whether anything actually reads `/metrics`, traces,
log level) and platform model keys.

Two judgments worth keeping:

- "Monitored" means something reads `/metrics`, not that it is exposed. The
  server notes each scrape; exposed but unread for five minutes is a warning.
- Stored secrets under a key id the server lacks are a problem (those
  secrets are lost without that key); under a known older key only a warning
  (rotation unfinished). A sample is decrypted too, to catch a key whose id
  stayed while its value changed.

## Planned

Worker version and self-reports (the registration message carries no
version yet), a table of server and egress-proxy replicas for version skew
and "NATS needed with several servers", the System → Health page, chaining
`everruns docker doctor` into it, an `everruns-server doctor` preflight, a
Prometheus gauge per check, SaaS checks and the release gate.
