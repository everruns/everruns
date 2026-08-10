# everruns-engine

> Deterministic, sans-I/O turn planning for Everruns execution hosts.

[![Crates.io](https://img.shields.io/crates/v/everruns-engine.svg)](https://crates.io/crates/everruns-engine)
[![Documentation](https://docs.rs/everruns-engine/badge.svg)](https://docs.rs/everruns-engine)
[![License](https://img.shields.io/crates/l/everruns-engine.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-engine` transforms serializable turn state, activity outcomes, and
host-resolved facts into the next turn plan and lifecycle effects. It performs
no storage, network, process, clock, or event I/O.

It is a focused implementation crate in the [Everruns](https://everruns.com)
ecosystem. Framework applications use `everruns`; runtime, worker, durable, and
custom hosts share this planner to keep turn semantics aligned.

## Quick Example

```rust
use everruns_engine::{TurnPlan, TurnState};

fn accepts_plan(_state: &TurnState, _plan: &TurnPlan) {}
# let _ = accepts_plan;
```

## What It Provides

- Serializable turn state and activity outcomes
- Pure next-turn planning functions
- Explicit host facts and lifecycle effects
- Shared semantics for in-process and durable execution hosts

## Documentation

- [Framework custom backends](https://docs.everruns.com/framework/custom-backends/)
- [API reference](https://docs.rs/everruns-engine)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
