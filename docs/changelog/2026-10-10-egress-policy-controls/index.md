---
title: "Outbound network policy modes, audit log and org allowlists"
description: "Operators choose open or curated outbound access, every outbound request is audited with its org and session, and admins can widen the allowlist for one org."
date: 2026-10-10
kind: feature
significance: notable
products: [platform, cloud, cli]
surfaces: [operations, cli]
links:
  docs: [/advanced/network-access/, /reference/cli/orgs/]
  prs: [everruns/everruns#4432, everruns/everruns#4437, everruns/everruns#4452, everruns/everruns#4476, everruns/everruns#4534]
---

Self-hosted deployments set `EVERRUNS_EGRESS_POLICY` to `open`,
`curated-writes` or `curated-all`. In `curated-writes`, plain GET and HEAD reads
reach public hosts while writes, MCP and integration traffic still need the
allowlist; a curated deny list applies first in both curated modes. The system
allowlist grew to about 510 host patterns.

Every outbound request is logged on the `everruns::egress::audit` target with
its decision, host, status, organization and session.

A platform administrator can let an organization extend the allowlist with up
to 50 host patterns, or grant it open egress. Manage this with
`everruns orgs egress-allowlist`.

See [Network access](/advanced/network-access/), the
[orgs CLI reference](/reference/cli/orgs/) and [#4432](https://github.com/everruns/everruns/pull/4432), [#4437](https://github.com/everruns/everruns/pull/4437), [#4452](https://github.com/everruns/everruns/pull/4452), [#4534](https://github.com/everruns/everruns/pull/4534).
