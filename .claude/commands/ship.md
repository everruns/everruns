# Ship

Run the full ship flow from AGENTS.md: validate, push, create PR, and merge when CI is green.

## Arguments

- `$ARGUMENTS` - Optional: description of what is being shipped (used for PR title/body context)

## Instructions

### 1. Pre-flight checks

- Confirm we're NOT on `main` or `master`
- Confirm there are no uncommitted changes (`git diff --quiet && git diff --cached --quiet`)
- If uncommitted changes exist, stop and tell the user

### 2. Rebase on latest main

```bash
git fetch origin main && git rebase origin/main
```

- If rebase fails with conflicts, abort and tell the user to resolve manually

### 3. Run `just pre-push` (fast ~30s)

```bash
just pre-push
```

- If it fails, run `just fmt` to auto-fix, then retry once
- If still failing, stop and report

### 4. Run `just pre-pr` (full validation)

```bash
just pre-pr
```

- If it fails, stop and report the failures

### 5. Push to remote

```bash
git push -u origin <current-branch>
```

### 6. Create PR (if none exists)

Check for existing PR first:

```bash
gh pr view --json url 2>/dev/null
```

If no PR exists, create one using the PR template (`.github/pull_request_template.md`):

- **Title**: conventional commit style from the branch commits
- **Body**: fill in the PR template sections (What, Why, How, Risk, Checklist) based on the actual changes
- Use `gh pr create`

If a PR already exists, report its URL and continue.

### 7. Wait for CI and merge

- Check CI status with `gh pr checks` (poll every 30s, up to 15 minutes)
- If CI is green, merge with `gh pr merge --squash --auto`
- If CI fails, report the failing checks and stop
- **NEVER** merge when CI is red

### 8. Post-merge cleanup

After successful merge:

- Report the merged PR URL
- Done

## Notes

- This codifies the "Shipping" section and "Pre-PR Checklist" from AGENTS.md
- Manual steps (smoke testing, UI screenshots) should be done BEFORE running `/ship`
- The `$ARGUMENTS` context helps generate a meaningful PR description
