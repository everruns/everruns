---
type: Test Case
title: "TC001: Query, save, export, and report roles"
description: "Verify that reports query only the current organization, support saved reports and CSV export, and gate management and projection controls by role."
tags:
  - everruns
  - test-case
  - ui
  - reports
---
# TC001: Query, save, export, and report roles

## Description

Verify that reports query only the current organization, support saved reports and CSV export, and gate management and projection controls by role.

## Preconditions

- The full stack is running with full authentication.
- Two organizations have distinct recent session or LLM-generation data markers.
- Owner, admin, and member users are available in the first organization.

## Test Data

| Field | Value |
|---|---|
| Route | `/reports` |
| Dataset | `LLM Generations` |
| Range | `7d` |
| Saved report | `EVE-995 generation report` |

## Steps

1. Sign in as the owner, open `/reports`, and select the dataset, dimension, measure, and range.
2. Confirm the Result table, Rows total, measure total, and Freshness values update for that query.
3. Verify no row or total includes the second organization's unique marker.
4. Enter the saved report name and click **Save report**.
5. Change the query, then run the saved report and confirm the original dataset, dimension, measure, range, and result return.
6. Export the current query and the saved report; open both CSV files and verify headers and rows match their displayed results.
7. As the owner, confirm **Backfill**, **Project**, Outbox, and Projection Lag are visible, and verify the Outbox summary shows its failed count.
8. If the failed count is greater than zero, confirm **Failed Rows** lists failure details; otherwise confirm the card is absent.
9. Delete the saved report and confirm it leaves the list.
10. Sign in as an admin and confirm report save/delete is available but owner-only projection controls and diagnostics are absent.
11. Sign in as a member and confirm querying and CSV export remain available while save/delete and owner-only controls are absent.
12. Switch between the two organizations and verify each report result and saved-report list changes to that organization's data.

## Expected Result

- Query controls produce matching org-scoped results and summary values.
- Saved reports preserve and rerun their stored query.
- CSV exports match the visible query and never include another organization's rows.
- Owners can administer projections, admins can manage saved reports, and members have read/export access only.
- Saved reports and diagnostics follow the selected organization.
