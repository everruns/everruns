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

### CHANGELOG.md as Source of Truth

1. CHANGELOG.md is the canonical source for release notes
2. GitHub Release notes are extracted from the corresponding version section in CHANGELOG.md
3. Each version section includes:
   - **Highlights** - Key features (user-written, 5-10 items with PR links)
   - **What's Changed** - List of commits: `- <message> ([#PR](url))`
   - **Migration Notes** - Breaking changes or upgrade instructions (if needed)

### Version Updates

The `/prepare-release` command updates version in:
- `Cargo.toml` (workspace.package.version)
- `apps/ui/package.json` (version field)
- `CHANGELOG.md` (new version section)

All packages (Rust crates and UI) are released together with the same version number.

### Lock File Updates

Lock files must be updated when preparing a release:
- `Cargo.lock` - Run `cargo update` to sync with new workspace version
- `apps/ui/package-lock.json` - Run `npm install` in apps/ui to regenerate

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
4. Docker images tagged with version (triggered by tag push)

### Docker Image Tagging

| Event | Tags Generated |
|-------|----------------|
| Version tag (v0.4.0) | `v0.4.0`, `latest`, SHA |
| Main branch push | `development`, SHA |
| Pull request | SHA only (no push) |

- **`latest`**: Only updated on version tags. Safe for production use.
- **`development`**: Tracks main branch. Updated on every commit to main.
- **SHA tags**: Always generated for traceability (short + full SHA).

### Tooling

- **git log**: Lists commits since last tag for changelog generation
- **GitHub Actions**: Auto-creates tag and release on merge
- **`/prepare-release` command**: Agent-invocable command for release preparation

## Non-Requirements

- No automatic version calculation (user specifies version)
- No release branches (releases from main only)
- No release candidates or pre-releases (can be added later if needed)
