---
type: Test Case
title: "TC003: Monitoring - Pause and replay"
description: "Verify pause and replay use observed population history without showing current ownership as historical detail."
---
# TC003: Monitoring - Pause and replay

## Description

Verify pause and replay use observed population history without showing current ownership as historical detail.

## Preconditions

- Org Monitoring is adopted and allowed, or a platform user can open fleet Monitoring.
- An active local fixture produces at least three distinct one-second observations.

## Test Data

| Input | Value |
|---|---|
| Population | Recorded local fixture IDs and API totals |
| Refresh deadline | Five seconds |

## Steps

1. Open Monitoring; record the live timestamp.
2. Select Pause and wait at least two seconds; compare the displayed timestamp and counts.
3. Resume live and verify observations continue.
4. Select Replay; choose an earlier timestamp and compare its counts with the recorded observation.
5. Verify Workers and Actors are unavailable during replay.
6. Resume live, open Density, and select a recorded sample.
7. Switch organizations and verify filters, actor detail, and prior-org history are discarded.

## Expected Result

Pause freezes one snapshot while collection continues. Replay contains observed summaries only. Resume restores live data; org switches reset retained presentation state.
