# Agents that start other agents

An agent starts other agents one of two ways. Pick one per agent; an agent
never has both.

| The agent should | Capability | Tools it gets |
|---|---|---|
| Call another agent once: a brief goes in, a result comes back, done | **Spawn agents** (`spawn_agents`) | `spawn_agent`, plus `get_task`, `wait_task`, `message_task`, `cancel_task` |
| Hand off work it follows, sends follow-ups to, and closes when accepted | **Threads** (`threads`) | `start_thread`, `message_thread`, `list_threads`, `get_thread`, `close_thread` |

Add **Coordinator** (`coordinator`) next to Threads when the agent should route
work to threads instead of doing it itself. It is a prompt only.

Every spawned agent and every thread can always talk to its parent: a thread
reports progress (`report_progress`), asks (`ask_parent`), finishes
(`finish`) or hands work back (`hand_back`); a spawned agent finishes with
`finish`. Children talk to each other only when the parent has **Sibling
communication** (`sibling_communication`). Every agent in one run shares the
`shared/` folder: pass file paths, not pasted content.

Choose:

- A review, a lookup, a check, parallel research: **Spawn agents**.
- Work the person will come back to, steer, or accept: **Threads**.
- An agent whose job is to run a team: **Threads** + **Coordinator**.

Both start the agent itself by default (`"self": true`, a fresh copy with its
own context). Name other agents in `agents` (by id; the model sees each by
name and description), or allow any agent of the organization with
`"any_agent": true`.

## Platform

```bash
# A builder that spawns a reviewer for one-off checks.
everruns agents update agent_… --capabilities '[
  {"ref": "spawn_agents", "config": {"self": true, "max_open": 3,
    "agents": [{"agent_id": "agent_…reviewer"}]}}
]' --reason "…" --context-revision <n>

# A coordinator that hands work to threads on itself and a copywriter.
everruns agents update agent_… --capabilities '[
  {"ref": "threads", "config": {"self": true, "max_open": 4,
    "agents": [{"agent_id": "agent_…copywriter"}]}},
  {"ref": "coordinator"}
]' --reason "…" --context-revision <n>
```

`--capabilities` replaces the whole list: start from `everruns agents get`.
Saving both Spawn agents and Threads on one agent is refused. The retired ids
`subagents`, `agent_handoff`, `coordination` and `run_conversation` are
rewritten into these on save.

## Framework

```rust
use everruns::threads::{Coordinator, Threads};
use everruns::{Agent, SpawnAgents};

// Spawn agents: one-off calls.
let lead = Agent::builder()
    .name("lead")
    .capability(SpawnAgents::new().max_open(3)) // optional: settings
    .spawns(reviewer)                           // the model passes agent: "reviewer"
    .build()?;

// Threads: followed work. Needs a local profile (`.local(LocalConfig::new(..))`).
let coordinator = Agent::builder()
    .name("launch-coordinator")
    .capability(Threads::new().max_open(4))
    .capability(Coordinator)
    .threads_with(copywriter)
    .build()?;
// everruns::threads::list(&session) reads the board.
```

Runnable examples on a real model: `crates/everruns/examples/spawn_agents.rs`
and `examples/coordinator-agent/` in the repository. Docs:
[Spawn agents](https://everruns.com/docs/capabilities/spawn-agents/) and
[Threads](https://everruns.com/docs/capabilities/threads/).
