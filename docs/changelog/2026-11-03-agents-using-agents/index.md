---
title: Agents that hand work to other agents
description: Give an agent other agents to hand work to. They work as one run with a shared team folder and bring every question back to one conversation.
date: 2026-11-03
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/capabilities/agent-handoff/, /framework/serve/]
  prs: [everruns/everruns#5175, everruns/everruns#5188, everruns/everruns#5196, everruns/everruns#5202]
---

An agent can now hand work to other agents. Add them on the agent's **Agents**
tab: an existing agent of your organization, or a new one made for it. Each
works as its own session in the same run.

Agents in a run share a `team/` folder, can start with a copy of the first
agent's files, and can message each other. Their approvals and questions reach
you in the conversation you are already in.

A session's **Agents** tab lists every hand-off with status, time and cost.
**Trace** and **Cost** switch to **Whole run**: one lane per agent, and the
spend split by agent.

In [serve](/framework/serve/), `.member("name")` hands work to another agent of
the app, replacing `#[agent(sub)]`:

```rust
#[agent(default)]
fn analyst() -> Agent {
    Agent::builder()
        .model("openai/gpt-6.1-sol")
        .member("reviewer")
        .build()
}
```

See [Agent handoff](/capabilities/agent-handoff/).
