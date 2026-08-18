---
type: Specification
title: "Release Process Specification"
description: "Release workflow with CHANGELOG.md."
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

Release readiness also includes the integration backstops that are intentionally kept off the `pull_request` hot path. Before cutting a release PR or merging it, review the latest push-only live integration workflow runs on `main` and the latest `.github/workflows/integration-live-sweep.yml` result. Do not release through unresolved failures there unless the failure is understood, documented, and explicitly accepted.

Release preparation also **audits the independently versioned library crates** (see [Library Crate Releases](#library-crate-releases)). This audit is a mandatory step of every product release, not an occasional side task: a product bump never carries a library crate's changes to crates.io, so any published crate whose public contract changed since its last release must get its own `/prepare-crate-release` — or the release PR must explicitly record that no published crate contracts changed. The decision must be visible in the PR, never silently skipped.

### CHANGELOG.md as Source of Truth

1. CHANGELOG.md is the canonical source for release notes
2. GitHub Release notes are extracted from the corresponding version section in CHANGELOG.md
3. Each version section contains:
   - **What's Changed** (required) - List of commits: `- <message> ([#PR](url))`
   - **Highlights** (optional) - Significant user-facing features and changes (user-written, with PR links). Include only items that are genuinely noteworthy on their own. Do not pad the list to hit a target count: maintenance releases may have few highlights, or omit the section entirely. Internal refactors, CI changes, dependency bumps, spec/docs updates, and minor fixes belong in **What's Changed**, not here.
   - **Crate Releases** (required) - Records the [library-crate release audit](#library-crate-releases) outcome in the changelog: the independently versioned crates published this cycle with their `old → new` versions, plus any crate deleted/absorbed this cycle and where its API moved. When the audit found no published crate contract changed, state that explicitly rather than omitting the subsection. This keeps the changelog a self-contained record of which crates.io versions correspond to each product release, and must match the release PR's audit note. When the GitHub Release notes were generated at tag time before crate versions were finalized, refresh the release body so the published notes carry the same list.

Release notes should not normally include a dedicated "Migration Notes" section. Migration-specific engineering detail belongs in the migration files and migration spec, which remain the source of truth for upgrade and database-migration behavior. If a release has any operator-visible migration caveat, compatibility limitation, or exceptional upgrade requirement, call it out explicitly in the release PR and release notes.

### Product Version Updates

The `/prepare-release` command updates the product version in:
- `Cargo.toml` (workspace.package.version)
- `apps/ui/package.json` (version field)
- `CHANGELOG.md` (new version section)

This version identifies the Everruns product: server, worker, CLI binaries, UI,
Docker images, and the GitHub release. Published Rust libraries own explicit
versions in their package manifests and are not bumped or published as a side
effect of a product release.

### Library Crate Releases

Each crates.io package is versioned and released independently.

**Audit obligation (every product release).** Independent versioning does not mean "ignore the
crates until someone complains." As part of preparing each product release, audit whether any
published crate's public contract changed since its last crates.io release and release the changed
ones. Guidance:

- Published crates are the workspace packages **without** `publish = false`. Diff each one's
  `src/` and manifest since its previous release.
- Judge *public contract*, not churn. Being git-touched over-counts massively — transitive
  dependency bumps, internal refactors, and pure directory moves (e.g. grouping providers under
  `crates/drivers/`) are not contract changes. A release candidate is a crate whose exported API,
  behavior, feature set, or MSRV changed, or a crate that was **deleted/absorbed** (its crates.io
  package is now orphaned and consumers must migrate to the new location).
- Record the outcome in the release PR — the crate releases cut, or an explicit "no published
  crate contracts changed" — so the decision is reviewable rather than assumed.

**Prefer the smallest compatible version bump.** Classify each changed crate breaking vs
non-breaking — run `cargo semver-checks --package <crate> --baseline-version <last-published>`, which
is authoritative, rather than eyeballing a diff. Then bump the minimum the change requires:

- Non-breaking change (only additions — new items, new modules, new variants added to a
  `#[non_exhaustive]` enum): bump the **patch** component (`0.18.0 → 0.18.1`).
- Breaking change (a removed or renamed public item, a changed signature, a tightened bound): bump
  the **minor** component (`0.18.0 → 0.19.0`) — for `0.x` crates the minor is the breaking slot.

Do not round every changed crate up to the product version for tidiness; a crate that only gained
API takes a patch bump even in a release where the product minor moved. `cargo-semver-checks` also
guards against under-bumping (shipping a breaking change as a patch), so run it on every crate you
release.

**A breaking bump is not done until its dependants are handled — the whole cone, not just the
crate.** A crates.io package is immutable, so a published dependant that pins the old, now-incompatible
requirement (`everruns-host = "^0.18.0"` when host is now `0.19.0`) will forever resolve the old
version, and a downstream consumer that also pulls the new one ends up compiling **two copies** — the
error surfaces inside the stranded upstream crate, not the consumer's code. This is the classic
partial release. For every crate that takes a **breaking** bump, close the cone:

- **Live dependants** whose own contract did not change still need a **patch bump and republish** so
  their new crates.io version pins the compatible requirement. (Optional/`dev`-only dependencies are
  lower urgency but should still be cascaded for a clean graph.)
- **Absorbed/deleted dependants** cannot be republished — **yank** their orphaned version with the
  **Yank Crate** workflow so new resolutions stop selecting them.

The **Crate Release** workflow's `strand-check` job enforces this: it fails the run when any
published crate's latest version pins a workspace crate at a requirement the current workspace
version no longer satisfies. A red `strand-check` means finish the cone (cascade-bump or yank) before
calling the release complete.

To release a changed crate:

1. Bump only the package whose public contract changed, by the smallest compatible increment above.
2. Run `python3 scripts/sync-publish-pin-versions.py --write` so published
   dependants reference the dependency package's current version.
3. Validate and merge the release change to `main`.

Tagging and publishing are then automated — the same schema as the product
release, where tags are created by CI rather than pushed by hand. On merge to
`main` the **Crate Release** workflow (`.github/workflows/crate-release.yml`)
compares each published crate's manifest version against crates.io, creates the
`crate/<package>/v<version>` tag for any version not yet published, and
dispatches **Publish Crate** for it. When several crates release together it
orders them by dependency so a dependant never publishes before the dependency it
pins is on crates.io. Detection is by crates.io presence, so a bump that merged
before the tag existed is still picked up and re-runs are idempotent (a version
already published is skipped). `workflow_dispatch` with `dry_run: true` previews
the set.

**Publish Crate** validates the selected manifest version, derives internal path
dependency pins from Cargo metadata, and publishes only that package. Manual
tagging remains a fallback when the workflow is unavailable, publishing
dependencies before dependants; do not recreate a lockstep workspace release.
Publishing cannot be completed from a sandbox whose egress policy blocks tag
pushes — the CI workflow is the supported path.

### Migration Handling

Release preparation does not squash feature migrations into a version-named file. Keep migrations as authored.

Do not rename, rewrite, or delete existing migrations just to align them with the release version. SQLx persists migration version, description, and checksum in `_sqlx_migrations`; changing a migration that may already have been applied breaks startup against existing databases.

Before merging a release PR, run the normal migration validation from `knowledge/operations/migrations.md`: filenames in `crates/server/migrations/` must remain strictly sequential with no gaps or duplicates.

If the release has an operator-visible migration caveat, compatibility limitation, or exceptional upgrade requirement, call it out explicitly in the release PR and release notes. Otherwise, do not add migration-specific release notes.

### Lock File Updates

Lock files must be updated when preparing a release:
- `Cargo.lock` - Run `cargo generate-lockfile` to sync product or crate version changes
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
4. Docker images tagged with version (triggered via `workflow_dispatch` from Release workflow)
5. Pre-built CLI binaries attached as release assets (triggered via `workflow_dispatch` from Release workflow)
6. crates.io packages are not published by the product release; each uses its
   own trusted package tag and **Publish Crate** workflow

> **Note:** Tags created by `GITHUB_TOKEN` don't trigger other workflows (GitHub anti-recursion).
> The Release workflow explicitly dispatches Docker Publish and CLI Binaries
> after creating the product release. Independent crate publishing validates a
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
