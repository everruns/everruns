//! Cells: in-memory, Raft-replicated session authorities for Everruns
//! Engine (data plane v2). Ported from funout `0a0c0ec`.
//!
//! A fleet is made of cells. A cell is a few nodes running one Raft group;
//! each session lives in exactly one cell as part of its replicated state
//! machine ([`state::CellState`]): the session's canonical Everruns event
//! log, its inbox and the claim naming the executor of its current turn. The
//! Raft log is the write-ahead log ([`wal::Wal`]); snapshots bound it.
//!
//! Each cell is an Everruns Engine hosting profile ([`hosting::RaftHosting`]):
//! admission, execution epochs, receipts and commands are Raft commands
//! applied by [`authority`], and executors run Engine's own session driver
//! (`Engine::activate`), so every step's events are committed on a quorum
//! before the turn moves on, and a turn cut off anywhere resumes from the
//! journal on another node. Execution is separate from leadership: a leader
//! change does not stop in-flight model calls. A directory (the state of
//! cell 0) maps sessions to cells, so capacity grows by adding cells, and
//! sessions can move.
//!
//! Design: `knowledge/execution/data-plane-v2/design.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod allowance;
pub mod api;
pub mod api_session;
pub mod authority;
pub mod batch;
pub mod cli;
pub mod cluster;
pub mod control;
pub mod dispatch;
pub mod dp;
pub mod edge;
mod edge_commands;
pub mod exec_api;
pub mod executor;
pub mod files;
pub mod gateway;
pub mod gateway_dp;
pub mod gateway_sse;
pub mod gateway_v1;
pub mod host;
pub mod hosting;
pub mod metrics;
pub mod moves;
pub mod network;
pub mod node;
pub mod node_control;
mod node_restore;
pub mod org_token;
pub mod prof;
pub mod prom;
pub mod providers;
pub mod remote;
pub mod restore;
pub mod restore_points;
pub mod ship;
pub mod sm;
pub mod state;
pub mod store;
pub mod stream;
pub mod tenants;
pub mod types;
pub mod wal;

pub use host::{Host, StandardHost};
