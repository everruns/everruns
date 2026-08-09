//! Shared effectful host orchestration for Everruns execution adapters.
//!
//! `everruns-host` sits between the pure `everruns-engine` planner and the
//! application, worker, and local adapters that apply its effects. It is a
//! transitive implementation boundary, not the ordinary application
//! entrypoint; applications in the [Everruns](https://everruns.com) ecosystem
//! should normally depend on `everruns`.
//!
//! The deprecated `everruns-runtime` package re-exports this implementation so
//! its 0.17 public paths continue to use the same canonical code.
//!
//! # Example
//!
//! ```
//! use everruns_host::{RuntimeHostAdapter, RuntimeHostTurnContext};
//!
//! fn accepts_host<A: RuntimeHostAdapter>() {}
//! fn accepts_context(_: RuntimeHostTurnContext) {}
//! # let _ = accepts_context;
//! ```

mod host;

pub use host::{
    RuntimeHostAdapter, RuntimeHostTurnContext, RuntimeSessionLifecycle, detect_dependency_blocker,
    execute_act_activity, execute_input_activity, execute_reason_activity,
    execute_reason_activity_with_prompt_messages,
};
