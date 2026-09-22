---
type: Test Case
title: "TC003: Linear MCP Actor Attribution"
description: "Verify Linear MCP user and service attachments, application-actor attribution, trigger execution, and revocation."
tags:
  - everruns
  - test-case
  - ui
  - agent-credentials
  - linear
---
# TC003: Linear MCP Actor Attribution

## Description

Verify the `catalog:linear` preset against Linear's remote MCP endpoint. A user
attachment must act as the invoking user. A service attachment must act as the
Linear application actor for attended and trigger-fired runs.

## Preconditions

- Run Everruns with a public HTTPS callback URL that Linear can reach.
- Use a disposable Linear workspace and application. Do not use production data.
- The Linear application actor is enabled and has `read,write` scopes.
- Create two Everruns users, Alice and Bob, who can invoke the same Agent.
- Create one Agent with both of these scoped attachments:

  - `linear_user`: `use: catalog:linear`, `actsAs: user`
  - `linear_service`: `use: catalog:linear`, `actsAs: service`

- Configure a trigger that makes the Agent create a uniquely named Linear issue
  through `linear_service`.
- Prepare an issue that Alice can comment on through `linear_user`.

## Steps

1. As Alice, connect `linear_user` and complete Linear consent.
2. Ask the Agent to add a uniquely named comment through `linear_user`.
3. As an org administrator, connect `linear_service` for the Agent and complete
   Linear consent.
4. As Alice, ask the Agent to create a uniquely named issue through
   `linear_service`.
5. As Bob, ask the Agent to create a second uniquely named issue through
   `linear_service`.
6. Fire the configured trigger and wait for its run to complete.
7. In Linear, open the comment and all three created issues. Record the remote
   author or attribution shown for each operation.
8. Capture screenshots of the Alice user-authored comment, both attended
   application-actor issues, and the trigger-created application-actor issue.
9. Revoke the Agent identity's Linear connection in Everruns.
10. Ask the Agent to create another issue through `linear_service`.
11. Capture the resulting connection prompt and confirm that no issue was
    created remotely.

## Expected Result

- The user attachment requests `read,write` without `actor=app`.
- Alice's user attachment operation is attributed to Alice in Linear.
- The service attachment requests `read,write` with `actor=app`.
- Alice's, Bob's, and the trigger-fired service operations are attributed to
  the same Linear application actor, not to either invoking user.
- Revocation takes effect on the next call. The call returns
  `connection_required`, and no cached identity token creates a remote issue.
- The screenshots show the remote attribution, unique test names, and the
  post-revocation prompt. Record the Everruns run IDs and Linear issue URLs with
  the test result.
