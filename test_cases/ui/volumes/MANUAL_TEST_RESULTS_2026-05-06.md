# Manual UI Test Results - 2026-05-06

## Environment

- **Auth Mode**: none (`AUTH_MODE` unset, dev defaults)
- **Stack**: `just start-dev --no-watch` (in-memory storage), API + Caddy + Next.js dev
- **PORT_PREFIX**: 271
- **Browser**: Chromium 1194 (headless, via agent-browser 0.26.0)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|-------------|--------|
| volumes  | 6     | 6    | 0           | 0      |
| **Total** | **6** | **6** | **0**       | **0**  |

## Detailed Results

### volumes (6/6 PASS)

- **TC001 Create Volume**: PASS — `tc001-research` created with description, `vol_` prefixed ID, `active` badge, "just now" timestamps.
- **TC002 Create Volume - Required Name Validation**: PASS — empty name blocked by HTML5 required validation; whitespace-only name blocked with inline `Name is required` error; no volume created on cancel.
- **TC003 Edit Volume Name and Description**: PASS — `tc003-source` renamed to `tc003-renamed`, description updated to `Renamed in TC003`, list and detail views both reflect the new values.
- **TC004 Archive Volume and Toggle Archived Filter**: PASS — confirmation dialog references `tc004-archive-me`; archived card hidden by default; `Show archived` filter reveals it with `archived` badge, disabled Edit, no Archive; detail page shows `archived` badge with Archived activity timestamp and disabled Edit.
- **TC005 Search Volumes by Name**: PASS — `alpha` search filtered list to `tc005-alpha`; clearing restored all three; `zzz-no-match` showed `No volumes found` without an in-state `New Volume` button.
- **TC006 Workspace Volumes Capability Mount Editor**: PASS — Workspace Volumes capability auto-added File System dependency; volume picker listed only active volumes (archived `tc004-archive-me` not selectable); `/etc/passwd` rejected with `Path must be /workspace or start with /workspace/`; two-row save persisted both mounts via API; `/workspace/research/docs` triggered overlap error on both affected rows; duplicate `/workspace/research` triggered `Duplicate mount path` error on both rows; trash icon removed the row.

## Issues Found

None.

## Evidence

Screenshots captured locally during the run (excluded from the repo):

- `/tmp/tc001_created.png`
- `/tmp/tc002_whitespace.png`
- `/tmp/tc003_renamed.png`
- `/tmp/tc004_confirm.png`, `/tmp/tc004_archived_filter.png`, `/tmp/tc004_detail.png`
- `/tmp/tc005_alpha.png`, `/tmp/tc005_nomatch.png`
- `/tmp/tc006_invalid_path.png`, `/tmp/tc006_two_mounts.png`, `/tmp/tc006_overlap.png`, `/tmp/tc006_duplicate.png`

## Notes

- Combobox volume selection in the mount editor required typing into the search input and pressing Enter to commit the selection (mouse-clicking the option in the listbox was also possible from the UI but was easier to drive deterministically via search + Enter under headless Chromium).
- Save Changes had to be invoked while the button was scrolled into view; clicking it from above the fold did not trigger the form submit reliably under headless Chromium. Manual users do not see this.
