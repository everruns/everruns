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

### CHANGELOG.md as Source of Truth

1. CHANGELOG.md is the canonical source for release notes
2. GitHub Release notes are extracted from the corresponding version section in CHANGELOG.md
3. Each version section contains:
   - **What's Changed** (required) - List of commits: `- <message> ([#PR](url))`
   - **Highlights** (optional) - Significant user-facing features and changes (user-written, with PR links). Include only items that are genuinely noteworthy on their own. Do not pad the list to hit a target count: maintenance releases may have few highlights, or omit the section entirely. Internal refactors, CI changes, dependency bumps, spec/docs updates, and minor fixes belong in **What's Changed**, not here.

Release notes should not normally include a dedicated "Migration Notes" section. Migration-specific engineering detail belongs in the migration files and migration spec, which remain the source of truth for upgrade and database-migration behavior. If a release has any operator-visible migration caveat, compatibility limitation, or exceptional upgrade requirement, call it out explicitly in the release PR and release notes.

### Version Updates

The `/prepare-release` command updates version in:
- `Cargo.toml` (workspace.package.version)
- `apps/ui/package.json` (version field)
- `CHANGELOG.md` (new version section)

All packages (Rust crates and UI) are released together with the same version number.

### Migration Handling

Release preparation does not squash feature migrations into a version-named file. Keep migrations as authored.

Do not rename, rewrite, or delete existing migrations just to align them with the release version. SQLx persists migration version, description, and checksum in `_sqlx_migrations`; changing a migration that may already have been applied breaks startup against existing databases.

Before merging a release PR, run the normal migration validation from `knowledge/operations/migrations.md`: filenames in `crates/server/migrations/` must remain strictly sequential with no gaps or duplicates.

If the release has an operator-visible migration caveat, compatibility limitation, or exceptional upgrade requirement, call it out explicitly in the release PR and release notes. Otherwise, do not add migration-specific release notes.

### Lock File Updates

Lock files must be updated when preparing a release:
- `Cargo.lock` - Run `cargo generate-lockfile` to sync with new workspace version
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
6. crates.io packages published from the trusted release commit (triggered via `repository_dispatch` from Release workflow)

> **Note:** Tags created by `GITHUB_TOKEN` don't trigger other workflows (GitHub anti-recursion).
> The Release workflow explicitly dispatches Docker Publish, CLI Binaries, and crates.io publishing after creating the release. Crate publishing validates that the tag is a strict semver tag, resolves to the expected release SHA when dispatched internally, and is reachable from `origin/main` before the crates.io token is used.

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
