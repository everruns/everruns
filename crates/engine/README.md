# everruns-engine

The sans-IO turn planner for the [Everruns](https://everruns.com) agentic
runtime.

`everruns-engine` holds the authoritative, deterministic turn-planning brain:
pure functions that take a serializable `TurnState` plus a parsed activity
outcome and return the next `TurnPlan` and the lifecycle effects the host must
perform. It does no I/O — no stores, sockets, process execution, event emission,
or `Utc::now()`. Hosts (in-process, durable, custom) resolve those facts, pass
`now` in, and apply the returned effects.

See `knowledge/foundations/sans-io-turn-state.md` for the design intent.
