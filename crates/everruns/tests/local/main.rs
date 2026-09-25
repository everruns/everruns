//! Single integration-test binary for the facade crate's `local`-feature
//! suites.
//!
//! Each `tests/*.rs` file is its own binary, and every binary links the whole
//! `everruns` crate from scratch (see `knowledge/project/ci-build-time.md`).
//! These suites all require `required-features = ["local"]` — without it they
//! fail to resolve `everruns_platform` — so they are merged into one target
//! declared once in Cargo.toml instead of one `[[test]]` entry per file.
//!
//! Run with: `cargo test -p everruns --features local --test local`
//! Run one module: `cargo test -p everruns --features local --test local <module>::`

mod local_composability_and_seam_test;
mod local_platform_store_test;
mod local_schedule_runner_test;
mod local_schedule_store_test;
mod local_spawn_agent_runtime_test;
mod local_task_registry_test;
mod workspace_environment;
