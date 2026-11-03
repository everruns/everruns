---
title: Agents that hand work to other agents
description: Give an agent other agents to hand work to. They work as one run with a shared team folder, talk to each other, and bring every question back to the person in one conversation.
date: 2026-11-03
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/capabilities/agent-handoff/, /framework/serve/]
  prs: [everruns/everruns#5175, everruns/everruns#5188, everruns/everruns#5196, everruns/everruns#5202]
---

An agent can now hand work to other agents. Open an agent's **Agents** tab and
add one: an existing agent of your organization, or a new one created for it.
The agent you talk to decides who gets which part, in the foreground or in the
background, and the others work as their own sessions in the same run.

The agents in a run share a `team/` folder that every one of them reads and
writes, so a triage note or a spec is a file the next agent opens. An agent can
also start with a copy of the first agent's files, and agents can message each
other while they work. When any of them needs an approval or has a question, it
reaches you in the conversation you are already in, with the asking agent's
name on the card.

**In the UI.** A session's **Agents** tab lists every hand-off with its
status, time and cost. **Trace** and **Cost** switch to **Whole run** to show
one lane per agent and the run's spend split by agent, and the organization's
Costs page shows what runs that use other agents cost, and who spent it.

**In code.** The [Framework](/framework/) adds members with
`AgentBuilder::member`, and [serve](/framework/serve/) with
`.member("name")` for another agent of the app:

```rust
#[agent(default)]
fn analyst() -> Agent {
    Agent::builder()
        .model("openai/gpt-6.1-sol")
        .member("reviewer") // another #[agent] of this app
        .build()
}
```

`.member("name")` replaces serve's `#[agent(sub)]` and its `ask_<name>` tool:
a member now runs as its own session in the run, with its own trace and cost.

See [Agent handoff](/capabilities/agent-handoff/).
