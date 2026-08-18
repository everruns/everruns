# Prepare Crate Release

Prepare one independently versioned crates.io package for release.
`$ARGUMENTS` contains the Cargo package and new version (for example
`everruns-core 0.18.1`). Ask for either value if missing.

Read [`knowledge/project/release-process.md`](../../knowledge/project/release-process.md)
before changing release mechanics.

## 1. Confirm scope

Use `cargo metadata --no-deps --format-version 1` to resolve the package and
confirm its manifest does not set `publish = false`. Review changes since its
latest `crate/<package>/v*` tag. A crate release does not bump the product
workspace, UI, or unrelated packages.

Choose the **smallest compatible version**. Run
`cargo semver-checks --package <package> --baseline-version <last-published>` to classify the change,
and take the minimum bump it allows: **patch** (`0.18.0 → 0.18.1`) for a non-breaking, additive-only
change, **minor** (`0.18.0 → 0.19.0`) only when there is a breaking change — the minor is the
breaking slot for `0.x` crates. Do not round up to the product version for tidiness; `cargo-semver-checks`
also fails a too-small bump, so it protects both directions. If `$ARGUMENTS` names a version that
disagrees with the tool, resolve the mismatch before continuing.

## 2. Update the package graph

Set the selected manifest's explicit `package.version`, then update published
dependants from the graph:

```bash
python3 scripts/sync-publish-pin-versions.py --write
python3 scripts/sync-publish-pin-versions.py --check
cargo generate-lockfile
```

Review every rewritten pin. If dependant crates require public changes rather
than a compatible dependency baseline update, release them separately after
the dependency package is available on crates.io.

## 3. Validate and merge

Run `just pre-push`, commit with
`chore(<package>): prepare vX.Y.Z`, and merge the normal PR after its checks are
green. Do not create the package tag from an unreviewed branch.

## 4. Let CI tag and publish

Tags are created by CI, not by hand — the same schema as the product release.
On merge to `main`, the **Crate Release** workflow (`.github/workflows/crate-release.yml`)
compares every published crate's manifest version against crates.io, creates the
`crate/<package>/vX.Y.Z` tag for any version not yet published, and dispatches
**Publish Crate** for it. When several crates are released together it walks them
in dependency order, waiting for each publish to finish before the dependant that
pins it. Detection is idempotent: a version already on crates.io is skipped.

Watch the Crate Release run to a green finish and confirm the new version appears
on crates.io. `workflow_dispatch` with `dry_run: true` lists what would be
released without tagging or publishing.

Manual tagging is a fallback only if the workflow is unavailable — at the
reviewed main commit, `git tag "crate/<package>/vX.Y.Z" <sha>` and push it, then
run **Publish Crate** with the package, tag, and SHA; publish dependencies before
dependants. Publishing to crates.io is not possible from a sandbox whose egress
policy blocks tag pushes; rely on the CI workflow there.
