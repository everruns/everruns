//! Observability exporters for Everruns hosts.
//!
//! This module implements the neutral observability contracts from
//! `everruns-core` with Braintrust and OpenTelemetry backends.
//!
//! ```rust
//! use everruns_host::observability::CompositeEventListener;
//! let _type_name = std::any::type_name::<CompositeEventListener>();
//! ```

pub mod braintrust;
pub mod composite;
pub mod otel;
pub mod telemetry;

pub use braintrust::{BraintrustConfig, BraintrustListener};
pub use composite::CompositeEventListener;
pub use otel::OtelEventListener;
pub use telemetry::{TelemetryConfig, TelemetryGuard, init_telemetry};
