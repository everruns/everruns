---
title: Agents start other agents with Spawn agents or Threads
description: Two clear ways for an agent to start others, never both on one agent. Spawn agents makes one-off calls; Threads hands off followed work.
date: 2026-11-25
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/capabilities/spawn-agents/, /capabilities/threads/, /framework/coordination/]
---

An agent now starts other agents one of two ways. [Spawn agents](/capabilities/spawn-agents/)
makes one-off calls: a brief goes in, a result comes back. [Threads](/capabilities/threads/)
hands off work the agent follows, sends follow-ups to and closes; add
Coordinator to make it route work instead of doing it. Saving both on one
agent is refused. Subagents, Agents, Coordination and Run conversation are
rewritten into these, and agents in one run share a `shared/` folder.

**In the UI**, the Team tab lists the agents either one may start.

**In code**, see [Agents that start agents](/framework/coordination/):

```rust
use everruns::{Agent, Model, SpawnAgents};

# fn main() -> Result<(), everruns::BuildError> {
let reviewer = Agent::builder()
    .name("reviewer")
    .instructions("Review the draft.")
    .model(Model::simulated("Looks good."))
    .build()?;
let lead = Agent::builder()
    .name("lead")
    .model(Model::simulated("Done."))
    .capability(SpawnAgents::new().max_open(3))
    .spawns(reviewer)
    .build()?;
# Ok(())
# }
```
