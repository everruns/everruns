//! File-backed session persistence for standalone library processes (EVE-836).
//!
//! The facade's default backends keep a session's conversation in memory, so a
//! process that exits loses the history. [`jsonl::JsonlSessionStore`] persists
//! that history as an append-only JSONL file and reloads it, letting a fresh
//! process resume a multi-turn session with only `everruns` and the `jsonl`
//! feature — no database, no platform store.
//!
//! It plugs in at the one seam the runtime already exposes: a session's message
//! store ([`everruns_runtime::RuntimeMessageStore`]). The store wraps the same
//! in-memory retriever the default runtime uses, mirrors every write to disk,
//! and seeds itself from the file on open — so resuming is exactly "seed the
//! message store, then run the next turn."

pub mod jsonl;

pub use jsonl::{JsonlError, JsonlSessionStore};
