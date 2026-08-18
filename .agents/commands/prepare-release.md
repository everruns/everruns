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

## 3. Audit published library crates for independent releases

Mandatory every release — never skip it, and never leave the outcome implicit. Published library
crates are versioned independently of the product, so a product bump does **not** carry their
changes to crates.io. If a crate's public contract changed and you don't cut its own release, the
crates.io package silently drifts behind its source.

1. Diff each published crate's source and manifest since its last release, e.g.
   `git diff "$PREV"..HEAD -- <crate-dir>/src <crate-dir>/Cargo.toml`. Published crates are the
   workspace packages **without** `publish = false`.
2. Judge *public contract*, not churn. "Touched" over-counts wildly: a transitive dependency
   bump, an internal refactor, or a pure directory move (like grouping the drivers under
   `crates/drivers/`) is **not** a contract change. A release candidate is a crate whose exported
   API, behavior, feature flags, or MSRV changed — or a crate that was **deleted/absorbed**, whose
   crates.io package is now orphaned and whose consumers must migrate.
3. For each crate whose contract changed, pick the **smallest compatible version bump**: classify it
   with `cargo semver-checks --package <crate> --baseline-version <last-published>` (authoritative,
   not an eyeballed diff), then bump the **patch** component for a non-breaking (additive-only) change
   (`0.18.0 → 0.18.1`) and the **minor** component only for a breaking change (`0.18.0 → 0.19.0`; the
   minor is the breaking slot for `0.x` crates). Do not round crates up to the product version for
   tidiness. Then run `/prepare-crate-release` to bump the package and run
   `python3 scripts/sync-publish-pin-versions.py --write`. Tagging and publishing are automated: on
   merge to `main` the **Crate Release** workflow (`.github/workflows/crate-release.yml`) creates
   `crate/<pkg>/v<ver>` and dispatches Publish Crate for any version not yet on crates.io, in
   dependency order — you never push crate tags by hand. For an absorbed/deleted crate, record where
   its API moved.
4. **Record the audit in the release PR**: either the list of crate releases cut this cycle, or an
   explicit "no published crate contracts changed" line. A reviewer must be able to see the decision
   was made, not assumed.

## 4. Update versions

- `Cargo.toml` → `workspace.package.version`
- `apps/ui/package.json` → `version`

Do not bump published library crate versions here. They are released independently with
`/prepare-crate-release` when their own public contracts change — that call is step 3, not this one.

## 5. Add the CHANGELOG entry

Insert after `## [Unreleased]`, preserving the file header and versioning policy:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Highlights

- **Feature Name** - Short description

### What's Changed

- feat: commit message ([#123](https://github.com/everruns/everruns/pull/123)) by [@username](https://github.com/username)

### Crate Releases

Independently versioned crates published this cycle:
- `everruns-<name>` A.B.C → X.Y.Z

Retired (absorbed — consumers migrate):
- `everruns-<gone>` → `everruns-<new-home>`
```

Link PRs and usernames. Add a **Migration Notes** section only when operators need upgrade guidance;
engineering-only migration detail belongs in `crates/server/migrations/` and `knowledge/operations/migrations.md`.
Include screenshot links for UI changes.

The **Crate Releases** subsection is required and records the step 3 audit outcome in the changelog
itself — the crates published this cycle with their `old → new` versions, plus any crate that was
deleted/absorbed and where its API now lives. When the audit found no published crate contract
changed, state that explicitly (`No published crate contracts changed this cycle.`) rather than
dropping the subsection. Keep it consistent with the release PR's audit note and, if the GitHub
Release notes were already generated at tag time, refresh that release body to match.

## 6. Verify migrations without rewriting them

Never squash, rename, or delete existing migrations for a release. Confirm the sorted basenames in
`crates/server/migrations/` still start at `001_` and stay strictly sequential
(`bash scripts/lib/check-migration-ordering.sh`).

## 7. Refresh lockfiles

```bash
cargo generate-lockfile
(cd apps/ui && pnpm install --lockfile-only)
(cd apps/docs && pnpm install --lockfile-only)
```

## 8. Commit and open the PR

Stage the files you touched by name, then:

```bash
git commit -m "chore(release): prepare vX.Y.Z"   # this message triggers auto-tagging on merge
git push -u origin <current-branch>
```

Open the PR with `.github/pull_request_template.md`, and tell the user to review CHANGELOG.md, add
any highlights or screenshots, and merge once CI is green — the tag, GitHub Release, Docker images,
and product binaries follow automatically. Crates.io publishing is independent: include the step 3
crate-release audit outcome in the PR body (the crate releases cut, or "no published crate contracts
changed"), so the decision is on the record rather than assumed.
