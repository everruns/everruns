---
type: Test Case
title: "TC005: Durable Schedules - Cron Validation"
description: "Verify five-field normalization, seven-field acceptance, invalid cron rejection, and unchanged persisted state after rejected updates."
tags:
  - everruns
  - test-case
  - api
  - scheduled-tasks
---
# TC005: Durable Schedules - Cron Validation

## Description

Verify five-field normalization, seven-field acceptance, invalid cron rejection, and unchanged persisted state after rejected updates.

## Preconditions

- Canonical authenticated stack with a platform user and a registered disposable target.
- Use paused schedules for syntax checks so accepted sub-minute expressions do not generate load.
- Creation uses `target: {"type":"workflow","name":"<registered-target>","input":{}}` and a unique name per row.

## Test Data

| Input | Expected |
| --- | --- |
| `* * * * *` | Accepted; stored as `0 * * * * * *` |
| `0 */6 * * *` | Accepted; stored as `0 0 */6 * * * *` |
| `0 * * * * * *` | Accepted as seven fields |
| `*/30 * * * * * *` | Accepted syntax; no retired minimum-interval claim |
| `* * * * * *` | 400; six fields unsupported |
| `bad`, empty, invalid numeric range | 400 |
| Unsupported target type | 400 |

## Steps

1. Create one paused schedule for each valid row and fetch it by returned ID.
2. Try invalid creates; compare list IDs before and after each attempt.
3. Patch a valid fixture with each invalid expression. Re-read it after every denial.
4. Patch it with a different valid five-field expression and verify the normalized persisted value.
5. Enable a once-per-minute fixture; verify its next-trigger time is populated. Pause and verify no next trigger is presented as active.

## Expected Result

- Accepted expressions normalize as listed; invalid requests return 400 and persist no partial change.
- Failed updates retain the previous cron, enabled state, and target.
- Enabling/resuming calculates a next trigger; paused syntax tests cause no executions.
- These cases cover parsing and lifecycle. They do not assert nonexistent per-org limits or a minimum interval setting.

## Source Contract

[Cron normalization](../../../../crates/server/src/domains/schedules/queries.rs).
