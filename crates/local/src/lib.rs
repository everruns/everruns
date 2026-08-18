//! Compatibility facade for the local in-process
//! [Everruns](https://everruns.com) backend.
//!
//! The local backend — SQLite-backed session catalog, task and schedule state,
//! git workspaces, and `LocalConfig` — was absorbed into the `everruns` facade
//! crate in 0.19 and now lives at [`everruns::local`]. This crate re-exports it
//! so existing `everruns-local` dependants keep compiling against
//! `everruns-host` 0.19 instead of resolving the old, now-incompatible 0.18
//! package (which pinned `everruns-host ^0.18`).
//!
//! New code should depend on `everruns` directly with
//! `default-features = false, features = ["local"]` rather than on this facade.
//!
//! ```no_run
//! use everruns_local::LocalProfile;
//!
//! // Re-exported unchanged from `everruns::local`.
//! let profile = LocalProfile::new("./agent-data");
//! # let _ = profile;
//! ```
pub use everruns::local::*;
