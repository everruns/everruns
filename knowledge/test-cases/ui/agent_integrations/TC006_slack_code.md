---
type: Test Case
title: "TC006: Slack Code shared tasks"
description: "Verifies Slack Code configuration, durable shared routing, review publication, requester approvals and lifecycle recovery."
tags: [everruns, test-case, ui, agent-integrations, slack]
---
# TC006: Slack Code shared tasks

## Description

Verifies Slack Code configuration, durable shared routing, review publication, requester approvals and lifecycle recovery.

## Preconditions

- A running Everruns stack with a coding agent and a real sandbox provider.
- A publicly reachable signed Slack webhook and a Code-enabled Slack installation.
- Two Slack users, a private source channel, an unrelated channel, and a restricted endpoint.
- Generated manifest installed with Code management and conversation inspection scopes.

## Test Data

Use a disposable repository and task: change a README, run its validation, publish
its diff, then ask before an outward-facing tool action.

## Steps

1. Open the Slack channel editor. Toggle **Slack Code channels**, save, reload, and verify persistence. Check the generated manifest contains Code enablement and scopes.
2. Mention the agent in the private source. Replay the signed event and simulate a server restart after remote creation; verify one task, channel and session. Verify the created channel remains private.
3. Have the second user post a top-level instruction and a thread reply. Verify both appear in the same session with their own actor attribution and use the same sandbox.
4. Mention the agent in the unrelated channel. Verify no task is created. Send a purported Slack-created channel with a different assigned bot; verify it is not adopted.
5. Publish context, summary and a real unified diff. Verify the Everruns link survives updates, an oversized diff is rejected, stale revisions cannot replace newer content, and a rate-limited update recovers after restart.
6. Request a consequential action. Verify a teammate cannot approve it; the requester can approve exactly once. Replay the click, expire a new request, and change its arguments; all must fail closed.
7. Stop through root-level and channel-wide controls. Restart the server and verify no old messages or projection calls resume. Send a fresh human instruction and verify it resumes the same task without old output.
8. Complete the task. Verify final summary precedes archive. Unarchive and verify the same session resumes.
9. Revoke the token and uninstall in separate runs. Verify execution cancellation and halted publication. Reinstall and verify ownership before resuming.
10. Disable Code on another endpoint. Verify ordinary mentions, threaded replies, DMs and agent-pane controls still work.

## Expected Result

Every task has one durable session; unrelated or unverified channels cannot bind
it. Review projections recover durably. Only the requester can approve the exact
pending action. Stops and revocation suppress output across restart.

## Notes

Automated fault and policy coverage lives in `crates/server/src/channels/slack/code/tests.rs`.
This case requires live Slack qualification; mocked API tests do not establish Slack's eligibility or stop contract.
