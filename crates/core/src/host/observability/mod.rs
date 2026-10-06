//! Observability exporters for Everruns hosts.
//!
//! This module implements the neutral observability contracts from
//! `everruns-core` with Braintrust and OpenTelemetry backends.
//!
//! ```rust
//! use everruns_core::host::observability::CompositeEventListener;
//! let _type_name = std::any::type_name::<CompositeEventListener>();
//! ```

#[cfg(feature = "braintrust")]
pub mod braintrust;
mod braintrust_delivery;
pub mod composite;
#[cfg(any(feature = "otel", feature = "braintrust"))]
mod generation_attrs;
#[cfg(feature = "otel")]
pub mod openinference;
#[cfg(feature = "otel")]
pub mod otel;
#[cfg(feature = "otel")]
mod otel_config;
#[cfg(any(feature = "otel", feature = "braintrust"))]
mod provider_attrs;
#[cfg(feature = "otel")]
pub mod telemetry;
#[cfg(feature = "braintrust")]
pub use braintrust::{BraintrustConfig, BraintrustListener};
pub use composite::CompositeEventListener;
#[cfg(feature = "otel")]
#[doc(hidden)]
pub use opentelemetry_sdk::trace::InMemorySpanExporter;
#[cfg(feature = "otel")]
pub use opentelemetry_sdk::trace::SdkTracerProvider;
#[cfg(feature = "otel")]
pub use otel::{OtelEventListener, TraceConventions};
#[cfg(feature = "otel")]
pub use telemetry::{TelemetryConfig, TelemetryGuard, init_telemetry};
