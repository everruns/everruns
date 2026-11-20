---
title: "Subagents can run on a different model"
description: "Agents send search to a cheap model, hard planning to a research model and review to another provider, by key, from a list the owner controls."
date: 2026-11-20
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/capabilities/sub-agents/]
---

A builder on GPT-6.2 Sol can now hand its diff to a verifier on another
provider, and its lookups to a small model. `spawn_agent` takes a `model`: one
of the agent's **helper models**, keys such as `fast`, `coding`, `deep` and
`second_opinion`, each with a line on when to use it. The agent never names a
raw model, so the owner decides what subagents cost and which providers see
the data.

![Agent page with the Helper models list: same, fast, coding, deep and second_opinion, and where each one's model comes from](./helper-models@2x.png)

Keys point at jobs, not models: `fast` follows the Small job of the agent's
preset or the organization defaults, `second_opinion` the Default job of a
provider other than the parent's. Edit the list under **Helper models** on the
agent page, add your own (a `gemini` key for a council of models), or switch
to **Auto** and let the Decision model pick a key from the task. Each
subagent's key, model and who picked it show in the session's **Subagents**
tab.

Subagents also stop forgetting the model you chose: a child now runs on the
model you picked for the parent chat instead of the agent's default. The list
lives in the Subagents capability config, so the API, MCP, the CLI and agent
packages carry it, see [Helper models](/capabilities/sub-agents/#helper-models).
