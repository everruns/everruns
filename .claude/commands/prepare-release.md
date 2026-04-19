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
   - Review the commit list and identify the 3-5 most impactful **user-facing** features and changes
   - Prioritize: new capabilities users can interact with, new integrations, significant UX improvements, security/reliability improvements that affect users
   - Deprioritize: internal refactors, CI changes, spec/docs updates, test additions (unless they enable a user-facing feature)
   - Present the commit list and your suggested highlights to the user
   - Ask the user to confirm, adjust, or replace the highlights
   - Note: Add markdown links for PRs `([#123](url))` and usernames `[@user](url))`

4. **After user approval**, update these files:

   **Cargo.toml** - Update `workspace.package.version`:
   ```toml
   [workspace.package]
   version = "X.Y.Z"
   ```

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

5. **Squash feature migrations into a version-named migration**:

   See `specs/release-process.md` ("Migration Squashing") and `specs/migrations.md`.

   - List `crates/server/migrations/*.sql` and find the last `NNN_vA.B.C.sql` file.
   - Identify every feature migration with a number strictly greater than that (these are the unsquashed feature migrations added since the last release).
   - If there are none, skip this step entirely — do not create an empty migration file.
   - Otherwise, create `NNN_vX.Y.Z.sql` (using the lowest number of the unsquashed set and the new release version) containing the concatenated SQL statements from those migrations, in their original numeric order, including any DDL/DML.
   - Preserve the origin of each block with an inline section header comment referencing the original filename, e.g. `-- from 016_eval_case_result_metadata.sql`.
   - Delete the original feature migration files so numbering stays strictly sequential with no gaps.
   - Verify the sorted list of migration file basenames starts at `001_` and remains strictly sequential with no gaps or duplicates (matching `specs/migrations.md`).

6. **Update lock files**:
   ```bash
   # Regenerate Cargo.lock with new workspace version
   cargo generate-lockfile
   # Regenerate package-lock.json
   cd apps/ui && npm install --package-lock-only
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
- The Highlights section is optional but recommended for minor/major releases
- Include screenshots for UI changes (can be added as links in CHANGELOG.md)
- Do not add a "Migration Notes" section to release entries unless it is necessary for user-facing upgrade guidance
- The `chore(release): prepare vX.Y.Z` commit message triggers auto-tagging on merge
