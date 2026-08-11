//! Concrete HTTP transports for Everruns hosts.
//!
//! `everruns-http` implements the neutral runtime egress contract from
//! `everruns-core` without pulling a concrete HTTP transport into the execution
//! kernel.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and is selected
//! explicitly by hosted or advanced execution adapters.
//!
//! # Example
//!
//! ```
//! use everruns_http::DirectEgressService;
//!
//! let transport = DirectEgressService::new();
//! # let _ = transport;
//! ```

mod direct;

pub use direct::DirectEgressService;
