---
type: Test Case
title: "TC002: Opt-in compaction monitoring"
description: "Verify organization consent, bounded quality findings, human review and disabling monitoring."
tags: [everruns, test-case, ui, organization-settings]
---
# TC002: Opt-in compaction monitoring

## Preconditions

- Canonical DB-backed stack; owner/admin of a new organization
- A test session with a synthetic provider and a real compaction outcome
- No live paid provider calls required

## Steps

1. Open Organization settings. Confirm monitoring is off with zero budget.
2. Enable monitoring without saving. Reload and confirm it remains off.
3. Enable and Save. Reload and confirm the setting persisted.
4. Run the representative synthetic long-session flow until compaction completes.
5. Wait for the worker. Confirm operational and continuity results are separate;
   a zero-budget continuity assessment is Insufficient evidence and spend is zero.
6. Open the finding's trace, return and disagree with its assessment. Reload and
   confirm Review recorded.
7. Stage monitoring settings through the page Chat adapter. Verify no write
   occurs until Save changes.
8. Disable and Save; run another compaction and confirm no new assessment.
9. Check the card at mobile width, light and dark themes. Check member controls
   are read-only using an authenticated member or permission regression test.

## Expected Result

Consent persists only after Save. Findings link to the correct session, never
present missing evidence as success and do not alter the agent. Reviews persist.
Opt-out prevents queued/new judging. No layout overflow or unreadable controls.
