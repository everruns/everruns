//! Meta Model API provider driver for Everruns.
//!
//! Meta Model API serves Muse models through an OpenAI-compatible Responses
//! API. [`MetaChatDriver`] wraps the shared Responses protocol driver and adds
//! first-party endpoint defaults plus model discovery.

mod driver;

pub use driver::{META_DEFAULT_API_URL, MetaChatDriver, provider, register_driver};
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
