---
type: Test Case
title: "TC001: boxd Sandbox Template - Hibernate Keeps Processes"
description: "Verify that a boxd Sandbox Template with a region and size runs a session on a boxd machine, hibernates it when idle, and wakes it with a background process still running."
tags:
  - everruns
  - test-case
  - ui
  - boxd-sandbox
---
# TC001: boxd Sandbox Template - Hibernate Keeps Processes

## Description

Verify that the boxd managed target accepts a region and size, that a session
on it runs on a boxd machine in that region, and that the idle hibernation
keeps a background process alive across the pause.

## Preconditions

- Canonical stack running, LLM provider configured
- A boxd API key (`bxd_...`) for an org with `nyc` enabled
- **No** existing boxd connection in Settings → Connections

## Test Data

| Field | Value |
|-------|-------|
| Template | Runs in **boxd**, Region **New York**, Compute size **2 vCPU, 8 GiB**, Outbound network **Only listed domains**: `example.com` |
| Idle | Pause after 60 seconds |
| Message 1 | Run `nohup python3 -m http.server 8000 >/dev/null 2>&1 &` then `curl -s -o /dev/null -w '%{http_code}' https://example.com` and `curl -sS -m 5 https://www.wikipedia.org` |
| Message 2 | Run `curl -s -o /dev/null -w '%{http_code}' localhost:8000` |

## Steps

1. Settings → Connections: connect **boxd** with the key; verify a key without
   the `bxd_` prefix is refused before saving.
2. Create a Sandbox Template with the test data. Verify the Snapshot field is
   disabled while a size is chosen, and the size is disabled while a snapshot
   is entered.
3. Attach the template to an agent and start a session; send Message 1.
4. Verify `example.com` answers `200` and `wikipedia.org` fails.
5. In the boxd console, verify a machine named `er-...` exists in New York
   with the `everruns.session_id` label.
6. Wait for the idle pause; verify the machine shows as hibernated in boxd and
   the session's Sandbox panel shows it paused.
7. Send Message 2.

## Expected Result

| Check | Expected |
|-------|----------|
| Connection | Valid key accepted; malformed key refused |
| Machine | One `er-...` machine in `nyc`, size 2x8, isolated |
| Egress | Listed host reachable, others blocked |
| Idle | Machine hibernated, not deleted |
| Resume | Same machine wakes; `localhost:8000` answers `200` (the server started before the pause is still running) |
