---
type: Specification
title: "Generic Sandbox snapshot builder"
description: "Script-driven provider artifacts for Sandbox Templates, with isolated build compute, revision guards, bounded notes, and explicit network and account scope."
tags:
  - everruns
  - sandbox
  - design
---

# Generic Sandbox snapshot builder

## Decision

**Build snapshot** extends the existing Sandbox Template editor. Authors supply an
install script; Everruns creates fresh provider compute, runs the script, bakes a
filesystem artifact, launches fresh compute to verify the result, and offers
**Save new revision**. No feature flag or separate promotion lifecycle is involved.
Dev tools, video editors, and other software are script contents. Everruns does not
maintain an application catalog, package versions, or workload-specific toggles.

The first native adapter is Daytona. Other existing targets remain usable as
ordinary Sandbox Templates. E2B template building and Modal built-image launching
need their own qualified adapters; a paused machine is not automatically a
reusable image, and a Modal internal image ID is not a registry tag.

## Ownership

The [build domain](../../crates/server/src/domains/sandbox_templates/builds/mod.rs)
owns inputs, lifecycle, commands and orchestration. The
[storage implementation](../../crates/server/src/storage/backend/sandbox_builds.rs)
owns tenant queries and atomic artifact application. Existing
[template records](../../crates/server/src/domains/sandbox_templates/record.rs),
[resolver](../../crates/server/src/domains/sandbox_templates/resolution.rs),
[HTTP routes](../../crates/server/src/api/sandbox_templates.rs), and
[template UI](../../apps/ui/src/components/sandboxes/sandbox-template-build-panel.tsx)
remain the surrounding product contracts.

A build is distinct from a runtime Session. It receives no Session workspace,
personal profile, workload credentials, volumes, or runtime bootstrap. The named
Organization connection is the exact provider account used for creation,
verification, cleanup and subsequent launch. Personal and Agent account sources
remain supported for ordinary templates, but cannot start snapshot builds.
Provider credentials stay outside compute. Builds retain a temporary encrypted
copy of their accepted management binding outside the public JSON record, using
the existing encryption service and encrypted-column rotation registry. It is cleared on application or confirmed
cleanup. Replacing a named connection cannot move existing build resources to
another provider account. A replacement key is adopted for cleanup only after
proving access to a known immutable resource id; application checks the current
connection can access the artifact.
Authors must choose a software-only base; existing files in a supplied base
snapshot are copied, and the builder cannot establish that an arbitrary base is
free of credentials.
Do not interpret native process isolation as a generic keyless-secret transport.

## Lifecycle and resource safety

Build attempts retain their immutable source inputs, bounded execution output,
optional author-supplied software notes, remote identities and final artifact.
The server's existing durable cluster job system runs and reconciles them. A
per-build PostgreSQL advisory lock prevents two server owners executing the same
script. Server interruption fails the attempt instead of replaying arbitrary
installation. Persist deterministic remote names before submitting create calls,
so a lost provider response still permits cleanup.

Only one queued or running build is accepted per organization, enforced by a
partial unique index. Processing across organizations is bounded, and overlapping sweeps are suppressed
per replica so long installs cannot occupy the shared maintenance pool. The total
execution deadline is independent of the bounded cleanup window. Cancellation,
timeout, failure and restart all enter cleanup. Provider deletion is confirmed;
uncertain snapshot submissions remain under reconciliation. Failed cleanup stays
visible and is retried. Provider TTLs bound abandoned compute when access is lost;
storage still depends on retaining provider access. Referenced Organization
accounts cannot be deleted while build resources, active template/Agent bindings
or runtime Sandboxes depend on them. Removing or revoking the credential outside Everruns can still require an
operator to restore access or remove resources at the provider.

Cleaned-up failed, cancelled and discarded history expires after thirty days;
applied provenance and billed unused artifacts remain retained.

Successful unused artifacts remain billed until discarded. Applying is an atomic
new immutable template revision, guarded by the build's source revision. A newer
edit, managed template, archived template, missing artifact, or incomplete cleanup
prevents application. Concurrent apply is idempotent. Applied artifacts cannot be
discarded because older Agents and Sessions may still pin them. Everruns never
moves those pins when a template's current revision changes.

## Installation, verification and runtime policy

Install scripts execute only in new provider compute. Existing
[bootstrap initialization](../../crates/capabilities/src/session_sandbox.rs) remains
launch-time setup and is neither copied into the build nor replaced by it.
Applying the result preserves the runtime account, containment, desktop settings,
durability, lifecycle and bootstrap, replacing only the base artifact selection.

Build-network permissions use the existing typed network control but are separate
from runtime containment. Package repositories may be allowed during installation
without broadening the finished sandbox's network. Fresh verification uses the
runtime network, including provider enforcement. URL-specific and block-only
Session rules retain their existing
[provider projection limitations](../../crates/contracts/src/runtime/sandbox_egress.rs).
Do not promise equal enforcement across providers.

An optional verification command executes after installation and again in a new
sandbox from the completed snapshot. Success proves only the checks the author
wrote. Desktop targets also must start their display from the fresh snapshot.
Daytona accepts a baked snapshot together with desktop settings; resource shape
comes from that snapshot.

## Readable reference metadata

A script may write a bounded non-secret Markdown file. Everruns captures it with
the build and shows it as escaped author reference data, never trusted policy.
The file remains in the baked filesystem for shell/file inspection. Output and
notes have explicit byte limits in the
[record contract](../../crates/server/src/domains/sandbox_templates/builds/record.rs);
provider output is bounded before collection as well as before persistence.

Session inspection resolves provenance from the exact applied snapshot and
Organization account within the authenticated tenant. This survives complete-spec
copying into Agents and Sessions without accepting a user-authored verified badge
or trusting a mutable file. The existing
[Session response](../../crates/server/src/domains/sandbox_templates/types.rs)
carries the result. Notes cannot enable tools, grant access or weaken containment;
they are not inserted as system instructions. A changed account or artifact does
not inherit another build's notes.

[Bound Secrets](../security/bound-secrets.md) remain a separate HTTP/MCP contract.
Arbitrary native code can copy a raw value it can read, so reusable image building
provides no raw secret input or secret mount. Modal's development-grade GitHub
header injection is a separate provider-specific mechanism, not a generic builder
secret bridge. Private runtime cookies and personal-computer recovery are outside
this reusable-software action.

## Delivery and qualification

Acceptance covers a real provider install and fresh snapshot launch, script and
provider failure, cancellation and cleanup, tenant/account mismatch, stale source
revision, readable notes, and preservation of runtime configuration and existing
pins. The owning UI case is
[Snapshot builder](../test-cases/ui/sandbox_provider_accounts/TC002_snapshot_builder.md).
Build commands use the shared command, CLI and native dispatch catalog. Platform
Chat's existing text-only template draft does not gain automatic script execution.

The user requires a video walkthrough review before YouTube publication or public
docs embedding. Merge, release and CI success do not constitute that approval.
The review cut and publication package stay private until approval.
