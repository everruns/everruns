//! Single integration-test binary for the in-memory `TestServer` contract
//! suites that ride along with `--lib` in the server CI shard.
//!
//! Each `tests/*.rs` file is its own binary, and every binary links the whole
//! `everruns-server` crate from scratch (see `knowledge/project/ci-build-time.md`).
//! These suites need no PostgreSQL and run in parallel (no
//! `--test-threads=1`), so they are merged into one target and run alongside
//! the crate's unit tests in the same `cargo test` invocation.
//!
//! Run with: `cargo test -p everruns-server --lib --test contracts`
//! Run one module: `cargo test -p everruns-server --lib --test contracts <module>::`

#[path = "../test_harness.rs"]
mod test_harness;

mod mcp_form_elicitation_test;
mod mcp_url_consent_test;
mod mcp_url_elicitation_test;
mod question_answers_test;
mod sse_replay_test;
