# PR and merge

## Open or update the PR

- Push the branch, then `gh pr view --json url` to detect an existing PR, `gh pr create` otherwise.
- Title: Conventional Commits, under 70 characters.
- Body: copy the headings from [`.github/pull_request_template.md`](../../../../.github/pull_request_template.md)
  exactly and fill every applicable section — including Before/After proof, risk, the security
  review, and **Follow-ups**. Do not substitute ad-hoc headings, even for small changes.
- For the Before/After, attach evidence a skeptical reviewer can check, not just a claim — CLI/API
  output, logs, or metrics, and screenshots for UI changes (`apps/ui`). State explicitly when a
  change has no observable behavior.

## CI

- `gh pr checks` watches CI. Never merge red.
- CI opt-out labels (`ci:skip-*`) are listed in [`knowledge/project/shipping.md`](../../../../knowledge/project/shipping.md).
  They exist to save interim CI cycles, not to weaken merge evidence: before merge, remove any label
  suppressing CI affected by the diff and rerun CI on the final commit so the `CI Opt-Out Policy`
  job passes. Add and remove with `gh pr edit --add-label` / `--remove-label`.

## Review sweep

- Inspect the full conversation and reviewer state: `gh pr view --comments`,
  `gh pr view --json reviews,latestReviews`. Use `gh api graphql` if thread resolution is unclear.
- Address **every** comment — nits, low-confidence suggestions, COMMENTED-state notes, and bot
  output. Analyze the concern, then either change the code or reply explaining why the current code
  is correct. Reply on every thread with what you did, then resolve it.
- Async reviewer bots post late. After the final push and again after CI turns green, wait at least
  2 minutes and re-check for new comments.

## Merge

- `gh pr merge --squash`, manually, only after CI is green and the final sweep is clean. Do not use
  auto-merge — it bypasses the post-green sweep.
- Re-run `bash scripts/lib/check-migration-ordering.sh` immediately before merging.
- After merge, watch main CI for the merge commit. A failure there is an active regression: fix or
  revert promptly.
