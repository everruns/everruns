---
type: Test Case
title: "TC002: Applications - Consumer workspace"
description: Verify a published Application uses one identity across isolated Chats, Rooms and explicitly shared Memory.
---
# TC002: Applications - Consumer workspace

## Description
Verify a published Application uses one identity across isolated Chats, Rooms and explicitly shared Memory.

## Preconditions
Published Application with two Chats, one Room and shared organization Memory;
configured HTTPS/OIDC host; two admitted identities and one excluded identity.
Use full authentication for the API. The isolated local fixture may prime its IdP
JWKS cache, but must retain signature, audience and binding validation.

## Test Data

| Field | Value |
| --- | --- |
| Room post | Draft review starts here. |
| Reply | The outline is ready. |
| Memory file | /voice.md |

## Steps
1. Sign in as an admitted participant; verify the overview and all resource links.
2. Open both Chats in separate tabs; create one conversation each. Record session IDs.
3. Attempt reading the other Chat's session and another participant's session. Verify 404.
4. Post the Room message and reply. Verify server-selected participant attribution;
   an explicit `as_member` request must fail without creating a message.
5. Read shared memory. With read-only access, attempt writing and verify unchanged bytes.
6. Grant write access; edit a text file, reload and verify persistence. A sibling-host
   Origin must be denied without changing the file.
7. Exclude the participant or unpublish. The next request through the existing browser
   session must fail. An excluded identity cannot obtain a consumer credential.
8. With organization audience enabled, verify a current member reuses their default
   runtime identity; an external audience retains an Application-specific identity.

## Expected Result
One Application identity spans its resources without management authority. Chat
conversations remain private; Room history and Memory are shared only as configured.
Current audience and publication state govern subsequent requests.

Automated evidence: [domain boundaries](../../../../crates/server/tests/domain/applications_test.rs)
and [real local host acceptance](../../../../scripts/test-applications-local.py).
