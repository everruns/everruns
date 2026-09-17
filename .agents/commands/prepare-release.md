# Prepare Release

Cut a release PR: changelog entry, version bumps, lockfiles.
`$ARGUMENTS` is the new version (e.g. `0.4.0`); ask for it if missing.

[`knowledge/project/release-process.md`](../../knowledge/project/release-process.md) owns the release contract — changelog
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

## 3. Confirm the single platform version holds

Everruns ships **one version for the whole platform**. Every published crate inherits it with
`version.workspace = true` and pins that same version for its internal dependencies, so a release
republishes the entire publish set at one number. There is no per-crate audit, no bump
classification, and no cone to close — those existed only because crates were versioned
independently.

What is left is one gate:

```bash
python3 scripts/sync-publish-pin-versions.py --check
```

It fails if a published crate declares a literal version instead of inheriting, if an internal pin
has drifted, or if a published crate depends on a private workspace package (the
`everruns-host` 0.23.0 failure). `--write` fixes the first two. CI runs the same check in the
**Lockfile** job.

Tagging and publishing stay automated: on merge to `main` the **Crate Release** workflow
(`.github/workflows/crate-release.yml`) creates `crate/<pkg>/v<ver>` for every published crate whose
version is not yet on crates.io and dispatches Publish Crate in dependency order. Under a single
version that is the whole publish set, every release. You never push crate tags by hand.

If a crate is **deleted or absorbed** this cycle, its crates.io package is orphaned: **yank** it with
the **Yank Crate** workflow and record where its API moved. That is the one crate-level judgement a
release still carries.

## 4. Update versions

- `Cargo.toml` → `workspace.package.version`
- `apps/ui/package.json` → `version`

That is the whole version change. Every published crate inherits `workspace.package.version`, so
bumping it moves all 41 crates.io packages at once — do not edit crate manifests. Internal pins in
`[workspace.dependencies]` carry the version literally and must move with it:

```bash
python3 scripts/sync-publish-pin-versions.py --write
```

The minor component is the breaking slot at `0.x`, and a release moves it for every crate whether or
not that crate changed. That is the deliberate trade: a version number no longer claims "this crate
changed" — `CHANGELOG.md` says that — in exchange for cascades and strandings being unrepresentable.

## 5. Add the CHANGELOG entry

Insert after `## [Unreleased]`, preserving the file header and versioning policy:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Highlights

- **Feature Name** - Short description

### What's Changed

- feat: commit message ([#123](https://github.com/everruns/everruns/pull/123)) by [@username](https://github.com/username)

### Crate Releases

All published crates ship at the platform version X.Y.Z.

Retired (absorbed — consumers migrate):
- `everruns-<gone>` → `everruns-<new-home>`
```

Link PRs and usernames. Add a **Migration Notes** section only when operators need upgrade guidance;
engineering-only migration detail belongs in `crates/server/migrations/` and `knowledge/operations/migrations.md`.
Include screenshot links for UI changes.

The **Crate Releases** subsection is required, but under a single version it is one line: every
published crate ships at the platform version. List only crates **retired or absorbed** this cycle
and where their API moved, since those are the packages consumers must migrate off. The
42-line `old → new` table that independent versioning required is gone.

## 6. Prepare the release card

Update `release-card.yml` for this release. Keep the card editorial: one short headline, a one-to-three-line
summary, and no more than three user-facing highlights. Every claim must already be supported by the release's
`CHANGELOG.md` section. Each highlight's `source` is a PR or commit marker that the renderer verifies in that
version's changelog section. Validate and render the reviewed card locally:

```bash
(cd apps/docs && pnpm run release-card --check --expect-version X.Y.Z)
(cd apps/docs && pnpm run release-card --expect-version X.Y.Z)
```

The rendered PNG is generated, not committed. The release workflow regenerates it from the tagged commit and
attaches it to the GitHub Release.

## 7. Verify migrations without rewriting them

Never squash, rename, or delete existing migrations for a release. Confirm the sorted basenames in
`crates/server/migrations/` still start at `001_` and stay strictly sequential
(`bash scripts/lib/check-migration-ordering.sh`).

## 8. Refresh lockfiles

Every lockfile that resolves a workspace crate records the platform version, including the
out-of-workspace ones — miss one and its `--locked` build fails against the new version.

```bash
cargo generate-lockfile
for d in crates/everruns/tests/fixtures/external-consumer evals/generic \
         evals/guardrail-calibration evals/platform-capability \
         examples/weekend-concierge-host; do
  (cd "$d" && cargo generate-lockfile)
done
(cd apps/ui && pnpm install --lockfile-only)
(cd apps/docs && pnpm install --lockfile-only)
```

## 9. Commit and open the PR

Stage the files you touched by name, then:

```bash
git commit -m "chore(release): prepare vX.Y.Z"   # this message triggers auto-tagging on merge
git push -u origin <current-branch>
```

Open the PR with `.github/pull_request_template.md`, and tell the user to review CHANGELOG.md, add
any highlights or screenshots, and merge once CI is green — the tag, GitHub Release, Docker images,
and product binaries follow automatically. Crates.io publishing follows the same tag: every
published crate is republished at the platform version, in dependency order. Call out any crate
retired or absorbed this cycle in the PR body, since that is the only crate-level decision left.
