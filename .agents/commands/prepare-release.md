# Prepare Release

Prepare a new release for Everruns. This command generates changelog entries, updates version numbers, and creates a PR for review.

See `specs/release-process.md` for the full release process specification.

## Arguments

- `$ARGUMENTS` - The new version number (e.g., "0.4.0")

## Instructions

1. **Validate version argument**
   - If no version provided, ask for one
   - Version should follow semver (e.g., 0.4.0)
   - Confirm the version makes sense (check current version in Cargo.toml)

2. **Generate commit list using git log**
   ```bash
   # Unshallow if needed (cloud agents often have shallow clones)
   git rev-parse --is-shallow-repository | grep -q true && git fetch --unshallow origin main

   # Find previous release: tag first, then chore(release) commit, then all commits
   PREV=$(git describe --tags --abbrev=0 2>/dev/null \
     || git log --oneline --grep='chore(release): prepare v' --format='%H' | head -1)

   # List commits since previous release (or all if no release found)
   if [ -n "$PREV" ]; then
     git log "$PREV"..HEAD --oneline
   else
     git log --oneline
   fi
   ```

3. **Analyze commits and suggest highlights**:
   - Review the commit list and identify only the genuinely impactful **user-facing** features and changes
   - Prioritize: new capabilities users can interact with, new integrations, significant UX improvements, security/reliability improvements that affect users
   - Exclude: internal refactors, CI changes, dependency bumps, spec/docs updates, test additions, minor fixes (unless they directly resolve a user-visible regression)
   - Do not pad the list to hit a target count. Maintenance releases (mostly fixes and internal work) may have one or two highlights, or none at all — in which case omit the Highlights section entirely
   - Present the commit list and your suggested highlights (or a recommendation to skip the section) to the user
   - Ask the user to confirm, adjust, or replace the highlights
   - Note: Add markdown links for PRs `([#123](url))` and usernames `[@user](url)`

4. **After user approval**, update these files:

   **Cargo.toml** - Update `workspace.package.version`:
   ```toml
   [workspace.package]
   version = "X.Y.Z"
   ```

   **Sub-crate path-dependency pins** - Several sub-crates pin sibling crates
   by exact version (e.g. `crates/core/Cargo.toml` pins `everruns-openui`,
   `everruns-a2ui`). These must match the workspace version
   or the `publish-crates` workflow rejects the tag. Run the helper after
   bumping the workspace version:

   ```bash
   python3 scripts/sync-publish-pin-versions.py            # rewrite drift
   python3 scripts/sync-publish-pin-versions.py --check    # CI-style verify
   ```

   The helper's pin map mirrors `dependency_versions` in
   `.github/workflows/publish-crates.yml`; keep both in lockstep when adding
   new path-pinned crates.

   **apps/ui/package.json** - Update `version`:
   ```json
   {
     "version": "X.Y.Z"
   }
   ```

   **CHANGELOG.md** - Add new version section after `## [Unreleased]`:
   ```markdown
   ## [X.Y.Z] - YYYY-MM-DD

   ### Highlights

   - **Feature Name** - Short description
   - **Feature Name** - Short description

   ### What's Changed

   - feat: commit message ([#123](https://github.com/everruns/everruns/pull/123)) by [@username](https://github.com/username)
   - fix: commit message ([#124](https://github.com/everruns/everruns/pull/124)) by [@username](https://github.com/username)

   <!-- Optional: keep only when the release needs user-facing upgrade guidance. -->
   <!--
   ### Migration Notes

   - Upgrade guidance, if needed.
   -->
   ```

5. **Review migrations without rewriting them**:

   See `specs/release-process.md` ("Migration Handling") and `specs/migrations.md`.

   - Do not squash, rename, rewrite, or delete existing migrations solely for release prep.
   - Verify the sorted list of migration file basenames starts at `001_` and remains strictly sequential with no gaps or duplicates (matching `specs/migrations.md`).
   - If a release includes an operator-visible migration caveat, note it in the release PR and release notes; otherwise skip migration-specific release notes.

6. **Update lock files**:
   ```bash
   # Regenerate Cargo.lock with new workspace version
   cargo generate-lockfile
   # Regenerate pnpm lockfiles
   (cd apps/ui && pnpm install --lockfile-only)
   (cd apps/docs && pnpm install --lockfile-only)
   ```

7. **Skip migration notes unless necessary**:
   - Do not add a dedicated migration section by default
   - Add one only when a release needs user-facing upgrade guidance
   - If migration behavior only needs engineering discussion, capture it in the canonical migration locations, for example `crates/server/migrations/` and/or `specs/migrations.md`, instead of the changelog

8. **Create commit**:
   ```bash
   git add -A
   git commit -m "chore(release): prepare vX.Y.Z"
   ```

9. **Create PR**:
   ```bash
   git push -u origin <current-branch>
   gh pr create --title "chore(release): prepare vX.Y.Z" --body "$(cat <<'EOF'
   ## Release vX.Y.Z

   This PR prepares the vX.Y.Z release.

   ### Checklist
   - [ ] Review CHANGELOG.md entries
   - [ ] Add highlights/screenshots if needed
   - [ ] Verify version numbers updated
   - [ ] CI passes

   ### Post-merge
   After merging, the release workflow will automatically:
   - Create git tag `vX.Y.Z`
   - Create GitHub Release with notes from CHANGELOG.md
   - Trigger Docker image build with version tag
   EOF
   )"
   ```

10. **Remind user**:
   - Review the PR, especially CHANGELOG.md
   - Add any highlights or screenshots by editing CHANGELOG.md
   - Merge when CI is green
   - Tag and GitHub Release will be created automatically

## Notes

- Always preserve the CHANGELOG.md header and versioning policy section
- The Highlights section is optional. Recommended for minor/major releases that ship genuinely noteworthy user-facing changes; omit entirely for maintenance releases
- Include screenshots for UI changes (can be added as links in CHANGELOG.md)
- Do not add a "Migration Notes" section to release entries unless it is necessary for user-facing upgrade guidance
- The `chore(release): prepare vX.Y.Z` commit message triggers auto-tagging on merge
