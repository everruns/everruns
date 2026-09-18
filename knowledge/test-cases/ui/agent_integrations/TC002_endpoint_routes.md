---
type: Test Case
title: "TC002: Agent endpoint full-page editors"
description: "Verifies the agent-scoped endpoint create and edit routes, including that they return to the Integrations tab rather than the App page."
tags:
  - everruns
  - test-case
  - ui
  - agent-integrations
---
# TC002: Agent endpoint full-page editors

## Description

Verifies the agent-scoped endpoint create and edit routes, including that they return to the Integrations tab rather than the App page.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated deployed Everruns UI is available
- The tester is signed in and has access to the target organization when authentication is enabled
- An agent exists

## Test Data

| Field | Value |
| --- | --- |
| Agent name | Dad Joke Agent |
| Endpoint type | webhook |

## Steps

1. Open the agent's Integrations tab.
2. Click **Add endpoint**.
3. Verify the URL is `/agents/{agentId}/endpoints/new` and the breadcrumb reads Agents › {agent} › New endpoint.
4. Choose the webhook type, complete the form, and save.
5. Verify the browser lands on `/agents/{agentId}/endpoints/{endpointId}`.
6. Verify the breadcrumb reads Agents › {agent} › Webhook channel — not Apps.
7. Change a field and click **Save**.
8. Verify the browser returns to `/agents/{agentId}?tab=integrations` and the row reflects the change.
9. Re-open the editor and click **Discard**.
10. Verify the browser returns to the Integrations tab without saving.
11. Navigate directly to `/agents/{agentId}/endpoints/does-not-exist`.
12. Verify a not-found page is shown whose back link returns to the agent's Integrations tab.

## Expected Result

- Endpoint create and edit are full-page routes under the agent, matching the existing full-page convention.
- Every exit path — Save, Discard, Back, not-found — returns to the agent's Integrations tab.
- An agent with no App can create an endpoint directly; the operation does not create or read an App.
