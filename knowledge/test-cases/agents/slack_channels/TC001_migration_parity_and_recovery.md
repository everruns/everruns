---
type: Test Case
title: "TC001: Slack migration parity and recovery"
description: "Verifies real-model Slack replies, requester-bound approvals and retained output across Channelkit cutover and recovery."
tags:
  - everruns
  - test-case
  - agents
  - slack
---
# TC001: Slack migration parity and recovery

## Description

Verifies real-model Slack replies, requester-bound approvals and retained output across Channelkit cutover and recovery.

## Preconditions

- An isolated canonical stack and designated Slack test app/conversation; record the exact build revision and channel, agent and organization identifiers.
- Local `AUTH_MODE=none` only, or an authenticated organization member with agent management permission; never use anonymous production access.
- A real OpenRouter `meta/muse-spark-1.3-contributor` agent with soft approvals, configured signing secret/workspace and checkpoint-capable store.
- A public HTTPS callback, or the authenticated Socket Mode supervisor with an app-level token. Record which transport actually receives provider-origin events.
- Two opted-in Slack test users for the allowed/denied approval checks; the second user's identity grants no Everruns authority.
- A separate controlled transport-fault fixture for uncertain writes. Never inject faults into another application's Slack traffic.

## Test Data

| Field | Value |
| --- | --- |
| Correlation | A fresh unique nonce for this run |
| First input | Mention the agent and ask it to return the nonce and calculate 137 + 256 |
| Approval input | Ask the agent to request consent with the nonce before reporting approval |
| Conversation | A new designated test thread, followed by the same thread after restart |

## Steps

1. Record an event cursor and the initial pending-delivery inventory. Publish the test channel, then send the first input as a real Slack user. Within two minutes, verify a new session/input and a completed real-model turn produce a Slack thread reply containing the nonce and correct calculation. Record message references and the final event cursor.
2. Send a follow-up in the same thread. Verify the original session and fractional Slack timestamps are preserved, prior context is available and no sibling installation's history or credentials are used.
3. Request approval. Record the completed requesting turn and card reference. Have the other user click first: verify no decision input or new turn appears and the approval audit/state is unchanged. Have the requester approve: verify one decision input starts the next turn in the intended session and exactly one org audit row names the verified Slack workspace, user, card, and decision. Replay the click and try a card from another channel/thread; neither may resume another turn or add an audit row. Repeat with private approvals and verify no public fallback exposes the card or resolution.
4. Exercise a pane stream through completion, then an interruption/replacement. Verify the original stream closes before replacement and no second stream or split progress fallback is left uneditable. Check a supported file, a denied/oversized file and a reaction in the designated thread; verify bounded intake and stored-message correlation.
5. In the controlled transport fixture, accept the first split part and fail the next ambiguously. Record the confirmed reference and pending output. Restart the host against the same store. Verify it retains the uncertain operation without reposting the accepted prefix or whole reply. Close only a known stream handle when applicable.
6. Seed an isolated unaudited legacy pending row with no ledger. Restart with the checkpoint-requiring adapter: verify the row is retained and no provider write or event replay occurs. Make its channel unavailable and repeat; the row must remain for reconciliation. Complete the reconciliation explicitly using the cutover runbook.
7. Repeat the server turn checks with v1 workers and data plane v2 in separate isolated stacks. Exercise the Framework/serve bridge with the same real model and checkpoint store. Label signed loopback requests as synthetic; they do not satisfy step 1's provider-origin evidence.
   On v2, render a pending card before the matched control-plane/shard upgrade,
   then send concurrent valid callbacks after upgrading. Verify exactly one
   decision input and completed turn. Replay after restart; neither the input nor
   provider output may repeat. Verify consumption survives event offload and
   snapshot restore even before a decision input is recorded, and an intervening
   input invalidates the checked log position. Missing canonical history or v2
   authority must fail closed without querying v1 approval events.
8. Unpublish the test channel, drain admitted turns and reconcile test-owned pending output. Remove only the run's own Slack messages/files and stop the test processes.

## Expected Result

- New source-correlated session events and provider readback prove the reply; an HTTP acknowledgement alone is insufficient.
- Conversation bindings, private placement, requester/channel/thread authorization, stream cleanup, history policy and host egress remain enforced.
- Unknown/partial acceptance retains every confirmed reference, and neither restart nor channel unavailability erases reconciliation evidence or blindly resends output.
- Record actual observations and deadlines separately from this acceptance case. Missing native-user or approval-click prerequisites remain an explicit qualification gap.
