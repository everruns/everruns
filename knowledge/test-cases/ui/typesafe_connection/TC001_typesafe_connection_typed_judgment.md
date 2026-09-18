---
type: Test Case
title: "TC001: TypeSafe Connection - Typed Classification"
description: "Verify that an agent with the TypeSafe capability prompts for an API key via Settings > Connections, validates it, and returns calibrated numbers from jev_evaluate rather than a prose opinion."
tags:
  - everruns
  - test-case
  - ui
  - typesafe-connection
---
# TC001: TypeSafe Connection - Typed Classification

## Description

Verify that an agent with the TypeSafe capability prompts for an API key via
Settings > Connections, validates it, and returns calibrated numbers from
`jev_evaluate` rather than a prose opinion.

## Preconditions

- Server running in a dev-grade deployment (`just start-all` recommended); the
  capability is `experimental_only` and is not registered in prod
- User logged in
- LLM API key configured
- No existing TypeSafe connection in Settings > Connections
- Valid TypeSafe API key available (`TYPESAFE_API_KEY` in Doppler)

## Test Data

| Field | Value |
|-------|-------|
| Capability | `[Experimental] Jev Classifications` |
| First Message | Rate this joke with jev_evaluate: "I told my wife she was drawing her eyebrows too high. She looked surprised." Ask whether it is a joke and how funny it is on a four-level scale. Report the numbers. |
| TypeSafe API Key | Valid key from typesafe.ai |

## Steps

1. Create an agent and enable the **[Experimental] Jev Classifications**
   capability. Save.
2. Start a session with the agent and send the first message.
3. Observe the tool call. **Expected:** the turn fails with a message naming
   `TYPESAFE_API_KEY` and pointing at Settings > Connections, because no
   connection exists yet. The turn is not wedged.
4. Go to **Settings > Connections**, find **TypeSafe**, click **Connect**, and
   paste an invalid key (e.g. `ts-nope`). **Expected:** validation rejects it
   with "Invalid API key", and no connection is stored.
5. Paste the valid key and connect. **Expected:** the connection saves and shows
   as connected.
6. Return to the session and resend the first message.

## Expected Results

- `jev_evaluate` succeeds and its result carries an `answers` object with
  one entry per question the agent asked.
- The yes/no answer is a `probability_yes` between 0 and 1; the graded answer
  carries `score`, `level`, `label`, `confidence`, and a `probabilities` map
  over the four levels.
- The maintenance-notice control (send the same request for "The quarterly
  maintenance window begins at 02:00 UTC on Saturday.") scores lower on both
  questions than the joke does.
- The agent's reply reports the numbers it got back rather than substituting its
  own impression.
- The API key never appears in the session transcript, tool arguments, tool
  results, or event payloads.

## Notes

- The capability's key is the **user's** connection. The deployment-owned
  `UTILITY_TYPESAFE_API_KEY` that backs guardrail `jev` checks is a separate
  credential and is not exercised by this test case.
