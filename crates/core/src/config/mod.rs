//! Shared configuration helpers for Everruns crates.
//!
//! Centralizes common environment-variable loading patterns so services,
//! workers, and other subsystems parse configuration the same way.
//!
//! The API is intentionally function-based. Config structs across Everruns are
//! heterogeneous, so composable helpers are clearer than a shared trait or
//! derive macro.
//!
//! # Example
//!
//! ```
//! use std::time::Duration;
//!
//! use everruns_core::config::{env_bool, env_duration_secs, env_or, env_string};
//!
//! let port = env_or("EVERRUNS_CONFIG_DOC_TEST_PORT", 8080_u16);
//! let bind_addr = env_string("EVERRUNS_CONFIG_DOC_TEST_BIND_ADDR", "127.0.0.1");
//! let enabled = env_bool("EVERRUNS_CONFIG_DOC_TEST_FEATURE_ENABLED", false);
//! let timeout = env_duration_secs(
//!     "EVERRUNS_CONFIG_DOC_TEST_TIMEOUT_SECS",
//!     Duration::from_secs(30),
//! );
//!
//! assert_eq!(port, 8080);
//! assert_eq!(bind_addr, "127.0.0.1");
//! assert!(!enabled);
//! assert_eq!(timeout, Duration::from_secs(30));
//! ```

mod env;
mod error;

pub use env::*;
pub use error::ConfigError;
