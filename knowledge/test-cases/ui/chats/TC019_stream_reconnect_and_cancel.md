---
type: Test Case
title: "TC019: Chats - Stream Recovery and Cancellation"
description: "Verify reconnect and reload recover one conversation without duplicate messages, and cancellation settles before a later turn."
tags:
  - everruns
  - test-case
  - ui
  - chats
---
# TC019: Chats - Stream Recovery and Cancellation

## Description

Verify reconnect and reload recover one conversation without duplicate messages, and cancellation settles before a later turn.

## Preconditions

- Canonical stack with real model and a disposable chat capable of a long response.
- Browser tooling can interrupt only the event-stream connection after the message request succeeds; restore it promptly.
- Record thread/session IDs, submitted message ID, and last event sequence. Use a two-minute recovery deadline.

## Test Data

| Turn | Message |
| --- | --- |
| Long response | Explain a moderately long topic in numbered paragraphs. |
| Follow-up | Reply with exactly `recovered`. |

## Steps

1. Open the chat, submit the long-response message once, and record its successful message response.
2. Interrupt its SSE stream while execution is active. Restore it and inspect reconnect requests and rendered messages.
3. Reload the same thread and compare transcript/message IDs with the stored API history.
4. Start another long response, wait until active, and choose Stop. Observe cancellation and settled lifecycle in Trace/Events.
5. Send the follow-up and wait for completion within the deadline. Reload once more.
6. Type an unsent draft, reload online and verify it restores. Submit another message with the network disconnected; record its client message ID, uploaded attachment references and local pending row.
7. Restore connectivity in the live tab. Verify exactly one accepted message under that identity. Repeat the failure, reload online, and verify the restored row waits for explicit Retry. Retry, then repeat using Discard and verify it only removes the local copy.
8. Switch organization or account and verify no previous draft or pending row renders. Log out and verify tab-local recovery is removed. Deny browser storage and verify unsent text stays editable with a visible recovery warning.
9. Open Chat and its Work/task view together. Confirm one active session stream is shared, then navigate away and confirm it closes. Restore network after retry exhaustion and confirm reconnect; an unmounted view must not reconnect.

## Expected Result

- Reconnection resumes from the last cursor and recovers events without resubmitting the user message or duplicating transcript entries.
- Reload preserves the same thread/session and committed conversation; there is no extra run created by reconnection.
- Cancellation reaches `turn.cancelled` then `session.idled`; send becomes usable once the session settles.
- The follow-up produces one completed turn and the expected response; late events do not leave the cancelled turn active or duplicate work.
- Unrecovered errors/timeouts are recorded as failures; a spinner or non-empty stale transcript is not a pass.
- Reloaded pending sends retain their identity and wait for Retry; live connectivity recovery does not replay server rejections, approvals or Stop.
- Drafts and pending attachment references remain within their account/org/session scope. Storage failure never claims durable recovery.
