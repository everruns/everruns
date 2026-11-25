---
title: Plugins for every agent, and for each person in Work Chat
description: Give an agent all approved plugins in one switch, let it load another when asked, and let each Work Chat user add or turn off plugins for themselves.
date: 2026-11-25
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/capabilities/plugins/]
---

Plugins are no longer an experiment. An admin approves them once in
**Settings > Organization > Plugins**, and an agent's new **Plugins** setting
takes *All approved* or a pick. A plugin approved next week joins every "all"
agent without anyone editing it. Turn on *Load others when asked* and a person
can say "set up Visti" mid-conversation.

![An agent's Plugins setting with All approved plugins selected, Stend and Visti included, and Load others when asked on](./agent-plugins@2x.png)

Work Chat users get a **Customizations** page. Each channel decides whether
they may add approved plugins or turn off one the agent includes:

```json
{
  "plugins": {
    "allow_adding": true,
    "allow_turning_off": false
  }
}
```

A plugin that needs a shell or sandbox the agent lacks is skipped instead of
failing the turn. See [Plugins](/capabilities/plugins/).
