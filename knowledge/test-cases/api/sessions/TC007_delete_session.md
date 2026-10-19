---
type: Test Case
title: "TC007: Delete Session"
description: "Verify that a session can be deleted and is no longer accessible."
tags:
  - everruns
  - test-case
  - api
  - sessions
---
# TC007: Delete Session

## Description

Verify that a session can be deleted and is no longer accessible.

## Preconditions

- Canonical local stack (see root `AGENTS.md`) or a dedicated test deployment is available
- Set `BASE_URL` to the selected proxy origin (for example, `http://localhost:27100`); requests below append `/api`
- For authenticated deployments, configure `curl` with the required authorization and organization headers

## Test Data

| Field | Value |
|-------|-------|
| Agent name | temp-agent |
| Agent prompt | Temporary. |

## Steps

1. Create agent:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/agents" \
     -H "Content-Type: application/json" \
     -d '{"name": "temp-agent", "system_prompt": "Temporary."}'
   ```
   Save `agent_id`.

2. Create session:
   ```bash
   curl -s -X POST "${BASE_URL}/api/v1/sessions" \
     -H "Content-Type: application/json" \
     -d '{"agent_id": "{agent_id}"}'
   ```
   Save `session_id`.

3. Delete session:
   ```bash
   curl -s -X DELETE "${BASE_URL}/api/v1/sessions/{session_id}"
   ```

4. Attempt to fetch deleted session:
   ```bash
   curl -s -o /dev/null -w "%{http_code}" "${BASE_URL}/api/v1/sessions/{session_id}"
   ```

## Expected Result

| Check | Expected |
|-------|----------|
| Step 3: HTTP status | 204 with no response body |
| Step 4: HTTP status | 404 |
