---
type: Specification
title: "Release Process Specification"
description: "Single-version release workflow with CHANGELOG.md."
tags:
  - everruns
  - project
---
# Release Process Specification

## Abstract

This specification defines the release process for Everruns. The process is designed to be coding-agent-friendly while keeping CHANGELOG.md as the single source of truth for release notes. Releases are triggered by asking an agent to prepare a release, which creates a PR. After review and merge, GitHub Actions automatically creates the tag and GitHub Release.

## Requirements

### Workflow

1. **Agent-driven release preparation**: User asks coding agent to release changes (e.g., "release the changes as 0.4.0")
2. **Agent runs `/prepare-release`**: Generates changelog, updates versions, creates PR
3. **User edits CHANGELOG.md**: Add highlights, screenshots, additional notes directly in the PR
4. **CI validates**: All checks must pass
5. **User merges PR**: Squash merge to main
6. **Auto-tagging**: GitHub Action detects release commit, creates tag + GitHub Release using CHANGELOG.md content

The release PR also carries `release-card.yml`, a concise presentation of facts already recorded in the
changelog. CI renders the card through the docs image toolchain for review. At tag time the Release workflow
regenerates the PNG from the trusted commit and attaches it to the GitHub Release. The card is presentation
metadata, not a second release-notes source: it cannot introduce claims absent from `CHANGELOG.md`.

Release readiness also includes the integration backstops that are intentionally kept off the `pull_request` hot path. Before cutting a release PR or merging it, review the latest push-only live integration workflow runs on `main` and the latest `.github/workflows/integration-live-sweep.yml` result. Do not release through unresolved failures there unless the failure is understood, documented, and explicitly accepted.

Everruns ships **one version for the whole platform** (see [Crate Publishing](#crate-publishing)). The product version and every crates.io package move together, so release preparation carries no per-crate audit: bumping `workspace.package.version` releases the entire publish set. The only crate-level decision a release still makes is recording any crate **retired or absorbed** this cycle, whose orphaned package consumers must migrate off.

### CHANGELOG.md as Source of Truth

1. CHANGELOG.md is the canonical source for release notes
2. GitHub Release notes are extracted from the corresponding version section in CHANGELOG.md
3. Each version section contains:
   - **What's Changed** (required) - List of commits: `- <message> ([#PR](url))`
   - **Highlights** (optional) - Significant user-facing features and changes (user-written, with PR links). Include only items that are genuinely noteworthy on their own. Do not pad the list to hit a target count: maintenance releases may have few highlights, or omit the section entirely. Internal refactors, CI changes, dependency bumps, spec/docs updates, and minor fixes belong in **What's Changed**, not here.
   - **Crate Releases** (required) - One line: every published crate ships at the platform version for this release. List only crates **retired or absorbed** this cycle and where their API moved, since those are the packages consumers must migrate off. Under a single platform version the crates.io version of every package is the release version, so the changelog stays a self-contained record without enumerating 40-odd `old → new` rows.

Release notes should not normally include a dedicated "Migration Notes" section. Migration-specific engineering detail belongs in the migration files and migration spec, which remain the source of truth for upgrade and database-migration behavior. If a release has any operator-visible migration caveat, compatibility limitation, or exceptional upgrade requirement, call it out explicitly in the release PR and release notes.

### Product Version Updates

The `/prepare-release` command updates the product version in:
- `Cargo.toml` (workspace.package.version)
- `apps/ui/package.json` (version field)
- `CHANGELOG.md` (new version section)

This version identifies the whole platform: server, worker, CLI binaries, UI,
Docker images, the GitHub release, **and every crates.io package**. Published
crates inherit it with `version.workspace = true` and never declare a version of
their own, so bumping `workspace.package.version` is the entire version change a
release makes. Internal pins in `[workspace.dependencies]` spell the version out
literally and are moved with `scripts/sync-publish-pin-versions.py --write`.

### Crate Publishing

Every crates.io package ships at the platform version. A crate manifest carries
`version.workspace = true`, never a literal version, and every internal path
dependency pins that same version. A release republishes the whole set.

**Why single-versioned.** Independent crate versions were the source of the
release failures, not a defence against them. `everruns-provider` is a transitive
dependency of 37 of the other 40 published crates and `everruns-model-profiles`
of 38, so at `0.x` — where the minor is the breaking slot — one breaking change
near the base republished almost the entire workspace anyway. Measured across
`0.19.0`–`0.28.0`, the last cycles each bumped essentially every published crate
while only a handful carried a real contract change: `0.27.0` was 8 of 41, and
`0.28.0` was 4 of 42 — the other 38 were pure cone re-pins. The independence was
nominal; the bookkeeping was not.

That bookkeeping is what failed. Cutting `v0.28.0` took two further full cascades
and three `fix(release)` commits inside a day — a stranded facade and 12
stranded dependants ([#3652](https://github.com/everruns/everruns/pull/3652)),
and a host that reached an immutable `0.23.0` tag it could not publish from
([#3656](https://github.com/everruns/everruns/pull/3656)).

The cost was not confined to release week, and it does not track consumer impact
at all. [#3659](https://github.com/everruns/everruns/pull/3659) settled on a name
for a contract nobody had adopted yet — `Judgment` became `Classifier` — plus
folded one crate into another. No consumer was using the old name, so the change
broke nothing in practice. But renaming an exported item in `everruns-core` is
breaking *by classification* at `0.x`, so the machinery charged full price: five
crates with real changes and patch bumps for 23 published dependants that changed
nothing, each a manifest edit, a pin, and a republish. That is the clearest
statement of the problem — the release cost was set by where the symbol lived,
not by what it cost anyone. Under a single version that release is a name and one
number.

**What a single version removes, structurally.** These are not checks that were
relaxed; they are failure modes that can no longer be expressed:

- **Stranding and partial cones.** Every crate pins the version every crate is
  published at, so a published dependant cannot be left pinning an incompatible
  predecessor. `check-publish-cone.py` and the Crate Release `strand-check` job
  are deleted.
- **Under-bumping.** Every release advances the `0.x` breaking slot, which is the
  largest bump any API change could demand. `cargo-semver-checks` had nothing
  left to classify — its own gate already skipped candidates that advance the
  breaking slot — so `check-semver-bumps.py` and the sharded **Crate Semver
  Bumps** job are deleted, along with ~45 minutes of rustdoc builds per release.
- **Cascade planning.** There is no per-crate bump to choose, so
  `plan-crate-release.py` is deleted.
- **Source drift at a published version.** The `--check-source-versions` guard
  existed to stop a crate's source changing while its version stayed put. A
  release now bumps every crate unconditionally, so a forgotten per-crate bump is
  impossible and the guard is gone. Between releases the tree carries the last
  released version while source moves on — exactly how `apps/ui/package.json`
  has always behaved.

**What it costs, deliberately.** A crate's version no longer claims "this crate
changed". `everruns-anthropic` moves with the platform whether or not it was
touched, and `CHANGELOG.md` is the record of what actually changed. Every release
is a breaking-slot bump for every consumer — which was already true in practice,
since recent cycles bumped everything regardless. All 41 packages are republished
each release; crates.io rate-limits this to roughly one publish per minute after
an initial burst, which the dependency-ordered **Crate Release** workflow absorbs.

**What single-versioning does not fix.** Three failure modes are orthogonal to
version choice and keep their guards:

- A published crate depending on a **private or registry-restricted** workspace
  package. This is [#3656](https://github.com/everruns/everruns/pull/3656), and
  `sync-publish-pin-versions.py` rejects it.
- A **new crate name** the registry token cannot publish, which fails mid-cascade
  and leaves a partial set. This is
  [#3648](https://github.com/everruns/everruns/pull/3648), and a bigger publish
  set makes it costlier, not cheaper. Confirm a new package name is publishable
  with the current `CARGO_REGISTRY_TOKEN` **before** merging the change that adds
  it; until then keep it `publish = false`.
- A crate **joining the publish set between releases**. Crate Release plans by
  crates.io presence, so a crate flipped off `publish = false` mid-cycle looks
  unpublished at the current platform version and is dispatched at once. It
  cannot succeed: its pins name that version, but the siblings already published
  at it came from the older commit, so cargo's verification build compiles the
  newcomer against stale dependency source. That is how
  `everruns-integrations-typesafe` 0.28.0 failed after
  [#3666](https://github.com/everruns/everruns/pull/3666) published it while
  0.28.0 was already cut and `everruns-core` 0.28.0 on crates.io predated the
  `ClassificationRequest::model` field it uses. The plan now defers a
  never-published crate to the next platform version, where every crate
  publishes from one commit again; `scripts/test-publish-crates-order.sh`
  exercises that against a stubbed index. Nothing is published wrongly either
  way, since the verification build fails closed, but the release run goes red
  until the version moves.

**Keep additive changes additive: `#[non_exhaustive]` on churn-prone public
types.** The cascade argument for this is gone, but the downstream one is not.
At `0.x` the minor is the breaking slot, so adding an enum variant or a public
struct field is a breaking change for **external consumers**: `LlmErrorKind`
gained a variant in `0.25.0` and another in `0.27.0`, and each hard-broke every
consumer's `match`. On a `#[non_exhaustive]` enum the compiler has already forced
a `_` arm, so the addition cannot break them when they upgrade.

The types carrying it are the ones with a demonstrated break, not every public
type: see [`LlmErrorKind`](../../crates/provider/src/error.rs), the two
[`ContentPart`](../../crates/core/src/message.rs) enums,
[`CapabilityStatus`](../../crates/core/src/capability_types.rs),
[`ModelCost`/`CostTier`](../../crates/model-profiles/src/types.rs),
[`LlmCallConfig`/`ProviderConfig`/`LlmCompletionMetadata`/`LlmStreamEvent`/`LlmContentPart`](../../crates/provider/src/driver_registry.rs), and
[`AgentAction`](../../crates/platform/src/audit.rs), which grows a variant whenever an
audited agent action is added, and whose two soft-approval variants were classified breaking
under the previous scheme for a change no consumer could observe.
Two consequences are deliberate:

- A `#[non_exhaustive]` **struct** cannot be built with a struct expression (or
  `..Default::default()`) from outside its crate, so each one owns a constructor -
  `new`, `for_provider`, or `Default` - and callers assign the public fields they
  need.
- Exhaustiveness checking is lost for *sibling workspace crates*, not just
  external consumers, so a new variant now falls into a `_` arm instead of
  failing the build. Every such arm says so and states what it does with an
  unrecognized value; they are unreachable in-workspace, where all crates compile
  against one version of the base crate.

Adding a **required trait method** is breaking for the same reason and is not
covered by `#[non_exhaustive]` (this is what `everruns-platform` `0.19.0` hit with
`SandboxCheckpointStore::rollback_current_checkpoint`). Ship a new trait method
with a default body unless breaking implementors is the point.

**`#[doc(hidden)]` is not a private boundary.** If a published crate calls an
item, that item is public contract whatever it is annotated with. The published
`everruns` facade drove a hidden steering surface on `everruns-host`
([`crates/host/src/runtime.rs`](../../crates/host/src/runtime.rs)) across a
crates.io boundary, which is the whole of
[#665](https://github.com/everruns/yolop/issues/665). That surface is documented
public contract now. Document such an item rather than hiding it.

**Retired and absorbed crates.** A single version cannot republish a package that
no longer exists. When a crate is deleted or absorbed, **yank** its orphaned
version with the **Yank Crate** workflow so new resolutions stop selecting it, and
record in the changelog where its API moved.

**The gate.** One check enforces all of this:

```bash
python3 scripts/sync-publish-pin-versions.py --check   # --write to fix drift
```

It fails on a published crate that declares a literal version, an internal pin
that does not match the platform version, or a published crate depending on a
private workspace package. CI runs it in the **Lockfile** job and again in
**Publish Crate** before the registry token is used.

**Publishing.** On merge to `main` the **Crate Release** workflow
(`.github/workflows/crate-release.yml`) compares each published crate's version
against crates.io, creates `crate/<package>/v<version>` for any version not yet
published, and dispatches **Publish Crate** for it in dependency order, so a
dependant never publishes before the dependency it pins. Detection is by
crates.io presence, so re-runs are idempotent and a version already published is
skipped. `workflow_dispatch` with `dry_run: true` previews the set.

**Publish Crate** validates the selected manifest version, derives internal pins
from Cargo metadata, and publishes only that package. Publishing cannot be
completed from a sandbox whose egress policy blocks tag pushes — the CI workflow
is the supported path.

### Migration Handling

Release preparation does not squash feature migrations into a version-named file. Keep migrations as authored.

Do not rename, rewrite, or delete existing migrations just to align them with the release version. SQLx persists migration version, description, and checksum in `_sqlx_migrations`; changing a migration that may already have been applied breaks startup against existing databases.

Before merging a release PR, run the normal migration validation from `knowledge/operations/migrations.md`: filenames in `crates/server/migrations/` must remain strictly sequential with no gaps or duplicates.

If the release has an operator-visible migration caveat, compatibility limitation, or exceptional upgrade requirement, call it out explicitly in the release PR and release notes. Otherwise, do not add migration-specific release notes.

### Lock File Updates

Lock files must be updated when preparing a release. Under a single platform
version every lockfile that resolves a workspace crate records that version, so
the out-of-workspace ones are not optional — a missed one fails its `--locked`
build against the new version:
- `Cargo.lock` - Run `cargo generate-lockfile`
- `crates/everruns/tests/fixtures/external-consumer/Cargo.lock`,
  `evals/generic/Cargo.lock`, `evals/guardrail-calibration/Cargo.lock`,
  `evals/platform-capability/Cargo.lock`,
  `examples/weekend-concierge-host/Cargo.lock` - Run `cargo generate-lockfile` in each
- `apps/ui/pnpm-lock.yaml` - Run `pnpm install --lockfile-only` in `apps/ui` to regenerate
- `apps/docs/pnpm-lock.yaml` - Run `pnpm install --lockfile-only` in `apps/docs` to regenerate

This ensures lock files reflect the current version and any dependency updates.

### Commit Convention

Release commits use: `chore(release): prepare vX.Y.Z`

This commit message triggers the auto-tagging workflow on merge to main.

### Manual Release Trigger

If the automatic release workflow fails, you can manually trigger it:
1. Go to **Actions → Release → Run workflow**
2. Enter the version number (e.g., `0.5.0`)
3. Click **Run workflow**

The workflow will extract release notes from CHANGELOG.md and create the GitHub Release.

### GitHub Release

1. Tag format: `vX.Y.Z` (e.g., `v0.4.0`)
2. Release title: `vX.Y.Z`
3. Release body: Extracted from CHANGELOG.md section for that version
4. Release card: `release-card-vX.Y.Z.png`, rendered from the reviewed `release-card.yml`
5. Docker images tagged with version (triggered via `workflow_dispatch` from Release workflow)
6. Pre-built CLI binaries attached as release assets (triggered via `workflow_dispatch` from Release workflow)
7. crates.io packages ship at the same version; each is published from its own
   trusted `crate/<package>/v<version>` tag by the **Publish Crate** workflow

> **Note:** Tags created by `GITHUB_TOKEN` don't trigger other workflows (GitHub anti-recursion).
> The Release workflow explicitly dispatches Docker Publish and CLI Binaries
> after creating the product release. Crate publishing validates a
> `crate/<package>/v<semver>` tag, the selected manifest version, the expected
> commit SHA, and reachability from `origin/main` before the crates.io token is
> used.

### CLI Binary Assets

The `Publish CLI Binaries` workflow builds and attaches pre-built CLI binaries to each GitHub Release:

| Asset | Target |
|-------|--------|
| `everruns-x86_64-apple-darwin.tar.gz` | macOS Intel |
| `everruns-aarch64-apple-darwin.tar.gz` | macOS Apple Silicon |
| `everruns-x86_64-unknown-linux-gnu.tar.gz` | Linux x86_64 |

Each archive contains the `everruns` binary. SHA-256 checksums (`.sha256` files) are included for verification. These assets are used by the Homebrew formula for installation.

### Homebrew Formula Update

After all CLI binaries are built and uploaded, the `Publish CLI Binaries` workflow automatically updates the Homebrew formula at `everruns/homebrew-tap`. It downloads the SHA-256 checksums from the release, generates an updated `Formula/everruns.rb`, and pushes it to the tap repository.

**Auth:** Uses the `HOMEBREW_TAP_GITHUB_TOKEN` PAT from Doppler (via `DOPPLER_TOKEN` secret), which has push access to `everruns/homebrew-tap`.

### Docker Image Tagging

| Event | Platforms | Tags Generated |
|-------|-----------|----------------|
| Version tag (`v*`) / `workflow_dispatch` | `linux/amd64` + `linux/arm64` | `vX.Y.Z`, `latest`, SHA |
| Pull request (Docker-relevant paths only) | `linux/amd64` | SHA |

- **`latest`**: Only updated on version tags. Safe for production use.
- **SHA tags**: Generated on every build for traceability (short + full SHA).
- **No `:development` tag and no per-main-commit images.** Docker images are a release artifact, not a per-commit artifact. Pin consumers to a released version (`vX.Y.Z` or `latest`).

#### Trigger rationale

Docker images are expensive to build, the slow path is `linux/arm64` via QEMU cross-compilation. Earlier versions of this workflow built on every push to `main` to keep a `:development` rolling tag, and on every PR to validate the build. That produced ~40–60 min of multi-arch build time per merge to main and ~18 min per PR, almost all of which was wasted when the change did not touch Docker infrastructure.

The current trigger shape (`docker-publish.yml`):

1. **Release-only publish.** Multi-arch images are built only on version tag pushes and the `workflow_dispatch` the Release workflow fires after creating the tag. The slow arm64 path runs a handful of times per week instead of on every merge.
2. **Path-filtered PR validation.** The workflow still runs on PRs, but only when Docker-relevant paths change (`docker/**`, `apps/ui/Dockerfile`, `apps/ui/.dockerignore`, `.dockerignore`, `.github/workflows/docker-publish.yml`). Rust/UI source changes are not validated per-PR; a broken Dockerfile will be caught by this gate, a source regression that only manifests inside the image is caught at release-tag time.
3. **No rolling main-branch tag.** Dropping `:development` removes the hidden-drift problem where `:development` could silently lag main (e.g., under a paths-filter) or produce images for every commit (expensive). Consumers that need a mainline image should build locally or use a released tag.

When a Dockerfile change is the *point* of a PR, the workflow runs and validates the amd64 build before merge. When the Dockerfile has not changed, the workflow is skipped entirely.

### Tooling

- **git log**: Lists commits since last tag for changelog generation
- **GitHub Actions**: Auto-creates tag and release on merge
- **`/prepare-release` command**: Agent-invocable command for release preparation

## Non-Requirements

- No automatic version calculation (user specifies version)
- No release branches (releases from main only)
- No release candidates or pre-releases (can be added later if needed)
