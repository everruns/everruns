---
title: Channel delivery cutover
description: Upgrade Slack delivery to Channelkit while preserving pending output and safe rollback.
---

Slack delivery uses host-owned output checkpoints with Channelkit provider
operations. The server schema upgrade is additive. Before changing delivery
implementations, account for pending output and stop every old delivery host.

1. Finish admitted turns and inspect the pending-delivery inventory. Record any
   unresolved provider operations and known message or stream references.
2. Reconcile legacy pending rows against provider state. A row created before the
   checkpoint protocol may already have sent output; the new host retains it
   without replaying the turn. Deleting a row or ledger and resending the reply is
   not reconciliation.
3. Stop all old delivery hosts before starting the new implementation. Keep a
   rollback build that understands and enforces delivery checkpoints. The shared
   database alone cannot make an older binary honor a protocol it does not know.
4. Start the upgraded hosts with the same installation identities, channel rows,
   credentials, session bindings and durable stores. Native file intake requires
   the configured host egress service.
5. Verify signed ingress, an existing conversation, completed replies, pane stream
   closure, reactions, private approvals and bounded file intake. Watch pending
   age, unknown/partial outcomes, duplicate posts and open streams.

An uncertain provider write remains pending until its outcome is reconciled. Only
known-handle idempotent cleanup may continue automatically. Rollback uses a
checkpoint-aware build; do not restart an older binary against checkpointed rows.

Output checkpoints do not promise provider-side exactly-once delivery or close the
existing inbound acknowledgement-before-background-acceptance crash window.
