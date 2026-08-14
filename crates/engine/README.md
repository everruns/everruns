# everruns-engine

> Shared Input/Reason/Act execution and sans-I/O turn planning for Everruns hosts.

[![Crates.io](https://img.shields.io/crates/v/everruns-engine.svg)](https://crates.io/crates/everruns-engine)
[![Documentation](https://docs.rs/everruns-engine/badge.svg)](https://docs.rs/everruns-engine)
[![License](https://img.shields.io/crates/l/everruns-engine.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-engine` owns the portable Input, Reason, and Act algorithms as well as
the deterministic planner that transforms serializable turn state, activity
outcomes, and host-resolved facts into the next plan and lifecycle effects.

Planning is pure. Execution effects cross injected contracts from
`everruns-core` and `everruns-provider`; the engine does not select a store,
transport, process runner, server, worker, platform, durable backend, or scale
deployment.

It is a focused implementation crate in the [Everruns](https://everruns.com)
ecosystem. Framework applications use `everruns`; runtime, worker, durable, and
custom hosts share this kernel to keep phase behavior, event ordering, and turn
semantics aligned.

## Quick Example

```rust
use everruns_engine::{TurnPlan, TurnState};

fn accepts_plan(_state: &TurnState, _plan: &TurnPlan) {}
# let _ = accepts_plan;
```

## What It Provides

- Serializable turn state and activity outcomes
- Pure next-turn planning functions
- Portable Input, Reason, and Act executors over injected contracts
- Engine-owned phase input/result types and concrete execution hooks
- Explicit host facts and lifecycle effects
- Shared semantics and event ordering for in-process and durable hosts

## Documentation

- [Framework custom backends](https://docs.everruns.com/framework/custom-backends/)
- [API reference](https://docs.rs/everruns-engine)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
