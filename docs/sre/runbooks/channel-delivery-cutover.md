---
title: Channel delivery cutover
description: Upgrade Slack delivery to Channelkit while preserving pending output and safe rollback.
---

Use this procedure when adopting Channelkit or another adapter that requires
host-owned output checkpoints. The schema upgrade is additive; the delivery
protocol also requires compatible owners.

1. Deploy the protocol-aware host baseline before enabling the new adapter.
   Every old adapter must retain protocol-marked pending rows even before a first
   intent creates their ledger. A binary that only checks for an existing ledger
   is not sufficient. Confirm all delivery hosts have reached the baseline.
2. Finish admitted turns and inspect pending output. Record unresolved provider
   operations and known message or stream references. Reconcile legacy rows
   against provider state: a row without a ledger may already have sent output.
   Deleting it and resending the reply is not reconciliation.
3. Stop or fence every incompatible old owner before starting the requiring
   adapter. Compatible owners must enforce the shared store's holder/lease fence
   on dispatch, renewal and completion; the shared database alone cannot make an
   older binary honor a protocol it does not understand.
4. Upgrade with stable installation/channel/session/binding IDs, credentials and
   durable stores. A requiring driver retains unmarked ledgerless legacy rows;
   legacy drivers retain marked rows. Missing or unpublished channels retain
   pending evidence for explicit reconciliation. Native file intake uses host egress.
5. Verify signed ingress, an existing conversation, completed replies, stream
   closure, reactions, private approvals and bounded file intake. Watch pending
   age, unknown/partial outcomes, duplicate posts and open streams.

Rollback uses the protocol-aware baseline or a later build that enforces the
same rules. Do not restart a ledger-only older binary against marked pending
rows. Unknown writes remain pending; only known-handle idempotent cleanup may
continue automatically.

Output checkpoints do not promise provider-side exactly-once delivery or close
the existing inbound acknowledgement-before-background-acceptance crash window.
