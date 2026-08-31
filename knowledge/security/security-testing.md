---
type: Specification
title: "Security Testing"
description: "Security testing process (threat-model tests, fail-rs, DeepSec, supply chain)."
tags:
  - everruns
  - security
---
# Security Testing

How Everruns verifies the mitigations claimed in `knowledge/security/threat-model.md`. Every
`MITIGATED` threat should be backed by at least one of the layers below, and new
features that touch a trust boundary must add coverage before they ship.

## Layers

### 1. Threat-model tests

Threats carry stable `TM-<CATEGORY>-<NNN>` IDs (see `knowledge/security/threat-model.md`).
Mitigations are exercised by integration and unit tests that assert the
boundary holds, cross-org isolation, auth rejection, permission enforcement,
input validation, error sanitization, channel signature verification, and so
on. Reference the threat ID in a code comment at the mitigation point using the
format in `knowledge/security/threat-model.md`:

```rust
// THREAT[TM-XXX-NNN]: Brief description of the threat being mitigated
// Mitigation: What this code does to prevent the attack
```

Tests for these live alongside the code they protect (for example
`crates/server/tests/auth_integration_test.rs`,
`crates/server/tests/mcp_endpoint_test.rs`, and the channel integration tests).

### 2. Failure injection (fail-rs)

The durable execution engine uses [fail-rs](https://github.com/tikv/fail-rs)
for fault injection against persistence and resilience paths. From a security
standpoint this verifies the engine degrades safely, no task hijack,
double-completion, or stuck state, when the database fails mid-operation
(TM-DURABLE, TM-DOS). Fail points are compiled out unless the `failpoints`
feature is enabled, so there is zero runtime overhead in normal builds.

The fail-point catalog, naming convention, test patterns, and the run command
live in [`knowledge/evaluation/fail-rs-testing.md`](../evaluation/fail-rs-testing.md), this spec does not
restate them. When adding a security-relevant failure path, add the fail point
and test there and reference the threat ID it guards.

### 3. AI-assisted code scanning (DeepSec)

`.deepsec/` holds a [deepsec](https://www.npmjs.com/package/deepsec) workspace
configured for the `everruns` project (priority paths: `apps/ui/`,
`crates/server/`, `crates/core/`, `crates/worker/`, `crates/host/`,
`integrations/`). It pairs a free regex `scan` with an AI `process` stage that
triages findings against the project context in `.deepsec/data/everruns/INFO.md`.

```bash
cd .deepsec
pnpm install
pnpm deepsec scan       --project-id everruns
pnpm deepsec process    --project-id everruns --concurrency 5
pnpm deepsec revalidate --project-id everruns --concurrency 5   # cuts false positives
pnpm deepsec report     --project-id everruns
```

`.deepsec/` is local dev-only scanning tooling, not a shipped dependency, so it
is exempt from the runtime seven-day dependency-maturity floor; bump it
(`pnpm update deepsec@latest`) in its own commit. Generated scan output is
gitignored; the curated `INFO.md` and config are checked in so context is shared. File real true positives as Linear/GitHub
issues with a `TM-` reference where one applies. See `.deepsec/README.md` for
setup and `.deepsec/AGENTS.md` for agent usage.

### 4. Supply chain and platform scanning

- **Licenses & advisories**: `cargo deny check licenses` and
  `cargo deny check advisories` (config in `deny.toml`), enforced by
  `.github/workflows/licenses.yml` on PRs that touch Cargo or crate files. The
  advisories step gates against RustSec vulnerabilities and yanked crates
  (vulnerabilities always fail; unmaintained advisories are not yet gated).
- **Dependency updates**: handled by Dependabot (`.github/dependabot.yml`),
  which covers the `cargo`, `docker`, and `npm` ecosystems, with security alerts
  reviewed in the GitHub Security tab.
- **Alert visibility**: `.github/workflows/security-alerts.yml` publishes only
  aggregate code-scanning and Dependabot alert counts into a job log daily; alert
  details remain protected by the security-alert API's access control. The alert APIs are
  unreadable from a scheduled agent session, so this is how such a session sees
  them; a workflow token reaches code scanning but not Dependabot alerts, which
  still need `security_events` on the GitHub App installation (EVE-926).
  Substituting a token does not help: the agent egress proxy rewrites
  `Authorization` for `api.github.com`, so a scheduled session authenticates as
  its own App installation no matter what it sends.
- **Secret scanning**: GitHub push-protection and open-alert review.
- **CI secret scoping**: fork-PR jobs never receive repository secrets
  (TM-CI-001..005); see `knowledge/security/threat-model.md`.

## When to add coverage

| Change | Required coverage |
|--------|-------------------|
| New trust boundary or auth/permission path | Threat-model entry + test |
| New persistence or task-lifecycle path | Fail point + failure injection test |
| New external channel/integration | Signature/replay/isolation tests |
| New dependency | Passes `cargo deny`; respects release-age floor |

## Related Files

- `knowledge/security/threat-model.md`, Threat catalog and mitigation status
- `knowledge/project/maintenance.md`, Security review cadence and release readiness
- `SECURITY.md`, Vulnerability disclosure policy
- `crates/durable/tests/failure_injection_test.rs`, Fail-point tests
- `deny.toml`, License and advisory policy
- `.deepsec/`, DeepSec scanning workspace
- `.github/workflows/licenses.yml`, Dependency license CI
