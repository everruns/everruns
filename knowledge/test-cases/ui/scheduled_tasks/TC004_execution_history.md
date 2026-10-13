---
type: Test Case
title: "TC004: Execution History and Filtering"
description: "Verify execution history preserves identifiers, terminal outcomes, error details, filtering, and workflow navigation."
tags:
  - everruns
  - test-case
  - ui
  - scheduled-tasks
---
# TC004: Execution History and Filtering

## Description

Verify execution history preserves identifiers, terminal outcomes, error details, filtering, and workflow navigation.

## Preconditions

- Disposable schedule with recorded successful and failed executions, plus pending/running/skipped fixtures where available.
- Caller has platform access; record expected execution IDs and results from the API.

## Test Data

| Fixture | Expected |
| --- | --- |
| Completed workflow | Workflow link and final duration |
| Failed execution | Error and terminal outcome |
| Filter with no matches | Explicit empty state |

## Steps

1. Open the detail, refresh Execution History, and compare its IDs/order against the current API response.
2. Inspect Status, Scheduled, Started, Completed, Duration, Workflow, and Error columns.
3. Hover relative timestamps and a truncated error; inspect exact timestamp/error tooltips.
4. Select Completed and Failed filters, then a status absent from the fixture; inspect requests and rows.
5. Return to All, open View Workflow, verify its input/output, and return with browser Back.

## Expected Result

- History agrees with the API; every displayed execution belongs to the selected schedule and filter.
- Terminal duration is in milliseconds; absent completion/duration/workflow values render as a dash rather than fabricated data.
- Failed rows expose their error, and a filter with no matches gives a clear empty state.
- Workflow links resolve to the correct workflow; returning preserves a usable schedule detail.
- Pagination buttons and live duration counters are not current UI promises. API pagination is covered separately.
