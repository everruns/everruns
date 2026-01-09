# Prepare Release

Prepare a new release for Everruns. This command helps generate changelog entries and update version numbers.

## Arguments

- `$ARGUMENTS` - The new version number (e.g., "0.4.0")

## Instructions

1. **Validate version argument**
   - If no version provided, ask for one
   - Version should follow semver (e.g., 0.4.0)

2. **Generate changelog draft using git-cliff**
   ```bash
   # Check if git-cliff is installed
   which git-cliff || cargo install git-cliff

   # Generate unreleased changes since last tag
   git cliff --unreleased --strip header
   ```

3. **Show the generated draft to the user** and ask them to:
   - Review which items are significant enough for the changelog
   - Identify any items that need rewording
   - Add any manual notes (migration warnings, breaking changes, etc.)

4. **After user approval**, update these files:
   - `Cargo.toml` - Update `workspace.package.version`
   - `apps/ui/package.json` - Update `version`
   - `CHANGELOG.md` - Add new version section at the top (after the header)

5. **Ask about migrations**:
   - "Does this release require database changes? Should migrations be squashed?"
   - If yes, help squash migrations as needed

6. **Create commit**:
   ```
   chore(release): prepare vX.Y.Z
   ```

7. **Remind user** to:
   - Create PR for review
   - After merge, create GitHub release with tag `vX.Y.Z`

## Notes

- Always preserve the CHANGELOG.md header and versioning policy section
- New versions go after the `## [Unreleased]` section (or create one if missing)
- Include the "no automatic migration" warning for minor/major versions with schema changes
