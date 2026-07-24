# Prepare Release

Cut a release PR: changelog entry, version bumps, lockfiles.
`$ARGUMENTS` is the new version (e.g. `0.4.0`); ask for it if missing.

[`specs/release-process.md`](../../specs/release-process.md) owns the release contract — changelog
structure, tagging, migration handling, and what happens after merge. Read it before deviating.

## 1. Collect the commits

```bash
# Cloud agents often have shallow clones
git rev-parse --is-shallow-repository | grep -q true && git fetch --unshallow origin main

# Previous release: tag first, then the release commit, else everything
PREV=$(git describe --tags --abbrev=0 2>/dev/null \
  || git log --oneline --grep='chore(release): prepare v' --format='%H' | head -1)

[ -n "$PREV" ] && git log "$PREV"..HEAD --oneline || git log --oneline
```

## 2. Propose highlights, then wait

Highlights are user-facing only: new capabilities, integrations, significant UX work, and
reliability or security improvements users feel. Internal refactors, CI, dependency bumps,
spec/docs, and tests belong in **What's Changed**. Do not pad to a count — a maintenance release may
have one highlight or none, in which case recommend dropping the section.

Present the commit list and your proposed highlights, and let the user confirm or replace them
before editing files.

## 3. Update versions

- `Cargo.toml` → `workspace.package.version`
- `apps/ui/package.json` → `version`
- Sibling path-dependency pins (`crates/core/Cargo.toml` pins `everruns-openui`, `everruns-a2ui`, …)
  must match the workspace version or the `publish-crates` workflow rejects the tag:

  ```bash
  python3 scripts/sync-publish-pin-versions.py            # rewrite drift
  python3 scripts/sync-publish-pin-versions.py --check    # CI-style verify
  ```

  Its pin map mirrors `dependency_versions` in `.github/workflows/publish-crates.yml`; keep both in
  lockstep when adding path-pinned crates.

## 4. Add the CHANGELOG entry

Insert after `## [Unreleased]`, preserving the file header and versioning policy:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Highlights

- **Feature Name** - Short description

### What's Changed

- feat: commit message ([#123](https://github.com/everruns/everruns/pull/123)) by [@username](https://github.com/username)
```

Link PRs and usernames. Add a **Migration Notes** section only when operators need upgrade guidance;
engineering-only migration detail belongs in `crates/server/migrations/` and `specs/migrations.md`.
Include screenshot links for UI changes.

## 5. Verify migrations without rewriting them

Never squash, rename, or delete existing migrations for a release. Confirm the sorted basenames in
`crates/server/migrations/` still start at `001_` and stay strictly sequential
(`bash scripts/lib/check-migration-ordering.sh`).

## 6. Refresh lockfiles

```bash
cargo generate-lockfile
(cd apps/ui && pnpm install --lockfile-only)
(cd apps/docs && pnpm install --lockfile-only)
```

## 7. Commit and open the PR

Stage the files you touched by name, then:

```bash
git commit -m "chore(release): prepare vX.Y.Z"   # this message triggers auto-tagging on merge
git push -u origin <current-branch>
```

Open the PR with `.github/pull_request_template.md`, and tell the user to review CHANGELOG.md, add
any highlights or screenshots, and merge once CI is green — the tag, GitHub Release, Docker images,
and crate publishes follow automatically.
