---
type: Test Case
title: "TC007: Investigate background work through existing platform commands"
description: "Verify Platform Chat can trace an alert to trigger runs, follow-up schedules and costs while preserving historical evidence and authorization."
tags: [test-case, platform-chat, costs, triggers]
---

# TC007: Investigate background work through existing platform commands

## Preconditions

- A disposable organization with a scheduled agent, a manual fire, a session
  follow-up and a newly opened background-work health issue.
- Current server and worker/agentd builds, with the investigation migration applied.
- An authorized owner/operator and a caller denied session access by the configured resolver.
- Repeat against both execution data planes.

## Steps

1. Ask Platform Chat to explain the health issue. Verify it reads the existing
   health issue command and distinguishes frozen opening evidence from current evidence.
2. Recheck the issue after its totals change. Ask for the original calculation;
   the opening window, thresholds and amounts must remain unchanged. Select an
   older episode after a reopen and verify the same evidence is returned.
3. Ask which fires were scheduled and which were manual. Page through run and
   delivery history, including records with equal timestamps. Verify no skipped
   or repeated IDs, and locate the input message of the manual fire in its session trace.
4. Ask why the session keeps waking. Verify the schedule list/get commands expose
   purpose, stop condition, lifetime, budget and stop reason, including deleted schedules.
5. Ask for costs grouped jointly by agent, session, source and starter. Filter to
   one session and compare the per-turn Cost view over an explicit time window.
   A host correction counts once; unpriced usage is reported as incomplete pricing.
6. Repeat the schedule reads as the denied caller and with a session from another
   organization. Verify refusal without schedule contents.
7. On v2, make the shard unavailable. Verify the read fails rather than substituting
   legacy PostgreSQL evidence. Historical unrecorded actors and pre-migration
   opening evidence must be described as unavailable.

## Expected result

Platform Chat cites the existing resources, explains the alert's own calculation,
and separates confirmed causes, hypotheses and missing evidence. It does not
invent an external provider bill or change schedules while investigating.

## Automated coverage

The `investigation_` tests in the owning domains cover frozen episodes, tenant and
parent checks, host permission resolution, stable trigger pagination, authenticated
manual actors, cost corrections and half-open windows. The control-plane data-plane
test verifies shard-authoritative schedule/cost reads; the portable cost projection
checks that private request content is absent from its wire response.
