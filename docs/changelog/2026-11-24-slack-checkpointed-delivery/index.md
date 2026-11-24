---
title: "Slack channels use checkpointed delivery"
description: "Slack delivery preserves accepted output across interruptions and uses Channelkit for provider operations."
date: 2026-11-24
kind: breaking
significance: notable
products: [framework, platform, cloud]
surfaces: [code, agent, operations]
links:
  docs: [/framework/slack-services/]
---

Slack delivery now records accepted message parts and stream handles before
continuing. An interrupted or partially accepted reply stays available for
reconciliation instead of being resent in full. Provider operations use Channelkit;
existing Slack apps, conversations and session bindings keep their identities.

**In Framework applications**, configure a stable installation and signing secret:

```rust
let slack = Slack::new(bot_token, signing_secret)
    .installation("support-slack")
    .workspace(team_id);
```

Custom `ChannelStore` implementations must support delivery checkpoints. The supplied
Memory, SQLite and PostgreSQL stores do. Host egress governs provider requests and
file downloads. Channel-wide and user-group notifications are neutralized in agent
output.

**For operators**, first deploy the protocol-aware compatibility baseline to every
delivery host and drain older owners, then roll out the Channelkit adapter.
Reconcile legacy pending rows and keep that compatibility baseline as the rollback
floor. See
[Slack services](/framework/slack-services/) and the
[delivery cutover runbook](/sre/runbooks/channel-delivery-cutover/).
