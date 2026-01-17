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

2. **Generate changelog draft using git-cliff**
   ```bash
   # Check if git-cliff is installed
   which git-cliff || cargo install git-cliff

   # Generate unreleased changes since last tag (or all if no tags)
   git cliff --unreleased --strip header
   ```

3. **Show the generated draft to the user** and ask them to:
   - Review which items are significant enough for the changelog
   - Identify any items that need rewording
   - Decide if highlights section is needed

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

   <!-- Add key highlights, screenshots, or notes here -->

   ### Added
   - Feature 1
   - Feature 2

   ### Changed
   - Change 1

   ### Fixed
   - Fix 1
   ```

5. **Ask about migrations**:
   - "Does this release include database schema changes?"
   - If yes, add migration notes to CHANGELOG.md and remind about fresh DB requirement

6. **Create commit**:
   ```bash
   git add -A
   git commit -m "chore(release): prepare vX.Y.Z"
   ```

7. **Create PR**:
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

8. **Remind user**:
   - Review the PR, especially CHANGELOG.md
   - Add any highlights or screenshots by editing CHANGELOG.md
   - Merge when CI is green
   - Tag and GitHub Release will be created automatically

## Notes

- Always preserve the CHANGELOG.md header and versioning policy section
- The Highlights section is optional but recommended for minor/major releases
- Include screenshots for UI changes (can be added as links in CHANGELOG.md)
- The `chore(release): prepare vX.Y.Z` commit message triggers auto-tagging on merge
