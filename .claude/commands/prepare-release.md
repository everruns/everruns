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

3. **Show the generated list to the user** and ask them to:
   - Review commits and confirm they look correct
   - Provide 3-5 highlights for the release
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

   ### Migration Notes

   **X.Y.Z → X.Y.Z:** Migration instructions if needed.
   ```

5. **Update lock files**:
   ```bash
   # Regenerate Cargo.lock with new workspace version
   cargo generate-lockfile
   # Regenerate package-lock.json
   cd apps/ui && npm install --package-lock-only
   ```

6. **Ask about migrations**:
   - "Does this release include database schema changes?"
   - If yes, add migration notes to CHANGELOG.md and remind about fresh DB requirement

7. **Create commit**:
   ```bash
   git add -A
   git commit -m "chore(release): prepare vX.Y.Z"
   ```

8. **Create PR**:
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

9. **Remind user**:
   - Review the PR, especially CHANGELOG.md
   - Add any highlights or screenshots by editing CHANGELOG.md
   - Merge when CI is green
   - Tag and GitHub Release will be created automatically

## Notes

- Always preserve the CHANGELOG.md header and versioning policy section
- The Highlights section is optional but recommended for minor/major releases
- Include screenshots for UI changes (can be added as links in CHANGELOG.md)
- The `chore(release): prepare vX.Y.Z` commit message triggers auto-tagging on merge
