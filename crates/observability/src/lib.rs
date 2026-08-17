//! Observability exporters for Everruns agents.
//!
//! This crate implements the neutral observability contracts from
//! `everruns-core` with Braintrust and OpenTelemetry backends. It is part of
//! the [Everruns](https://everruns.com) ecosystem.
//!
//! ```rust
//! use everruns_observability::CompositeEventListener;
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
