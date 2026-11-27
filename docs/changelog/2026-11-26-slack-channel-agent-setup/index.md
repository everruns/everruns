---
title: Slack channel agents go live on install and react on their own
description: Add to Slack now publishes the channel, Health flags a bot missing from its channel, and explicit agents get 👀 and ✅ reactions without prompt text.
date: 2026-11-26
kind: improvement
significance: minor
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
links:
  docs: [/capabilities/slack/, /features/explicit-communication/]
---

A finished **Add to Slack** now publishes the channel when the installer may
publish it, instead of leaving a draft. Health reports when the bot is not
a member of its bound channel and says to `/invite` it, which private channels
always need. See [Slack](/capabilities/slack/).

Agents that talk with `send_message` and have Reactions get 👀 when they start
real work and ✅ once they answered, with no prompt text. Their own reaction
replaces both. `send_message` takes `message`, like the other tools, and still
reads `text`; see [Explicit communication](/features/explicit-communication/).

```json
{
  "message": "Checking the open Dependabot PRs now."
}
```

CLI help names each command's output shape, and the new **Slack Channel
Coordinator** example agent runs on a schedule in one channel.
