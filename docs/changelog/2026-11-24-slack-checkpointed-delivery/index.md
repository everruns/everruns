---
title: "Slack channels use checkpointed delivery"
description: "Slack delivery preserves accepted output across interruptions, records confirmed message references, and uses Channelkit for provider operations."
date: 2026-11-24
kind: breaking
significance: notable
products: [framework, platform, cloud]
surfaces: [code, agent, operations]
links:
  docs: [/framework/slack-services/]
---

Slack delivery records accepted message parts and stream handles for reconciliation
after interruptions. Channelkit handles provider operations; existing Slack apps,
conversations and session bindings retain their identities.

**[Framework applications](/framework/slack-services/)** configure a stable installation and signing secret:

```rust
let slack = Slack::new(bot_token, signing_secret)
    .installation("support-slack")
    .workspace(team_id);
```

Custom `ChannelStore` implementations must support checkpoints. The supplied Memory,
SQLite and PostgreSQL stores do. Host egress governs provider requests and file
downloads. Agent output neutralizes channel-wide and user-group notifications.
Slack files reach Responses models. Data plane v2 approval decisions remain
single-use across shard restarts.

**Operators** deploy the protocol-aware baseline to every delivery host and drain
older owners before enabling the adapter. Reconcile legacy pending rows and retain
the rollback floor. On v2, pause writes and upgrade the control plane and shards
together; rollback requires a pre-upgrade restore point. See the
[cutover runbook](/sre/runbooks/channel-delivery-cutover/).
