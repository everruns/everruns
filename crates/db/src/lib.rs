//! Database utilities for the [Everruns](https://everruns.com) agent framework.
//!
//! - [`UpdateField`]: a typed "unchanged, clear, or set" field for update
//!   operations, shared by storage code that has no other common dependency.
//! - [`sqlite`] (feature `sqlite`): opening embedded SQLite databases,
//!   consistent backups, and SQLite's process-wide VFS selection. Hosts keep
//!   their own schemas and query callbacks; construction lives here.
//!
//! This crate depends on no other Everruns crate, so the durable engine and
//! hosts can share it without pulling each other in.
//!
//! ```rust
//! use everruns_db::UpdateField;
//!
//! let mut title = Some("draft".to_string());
//! UpdateField::Set("final".to_string()).apply(&mut title);
//! assert_eq!(title.as_deref(), Some("final"));
//!
//! UpdateField::<String>::Clear.apply(&mut title);
//! assert_eq!(title, None);
//! ```

#[cfg(feature = "sqlite")]
pub mod sqlite;
pub mod update_field;

pub use update_field::UpdateField;
