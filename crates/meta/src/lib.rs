//! Meta Model API provider support for the
//! [Everruns Framework](https://docs.everruns.com/framework/).
//!
//! Meta Model API serves Muse models through an OpenAI-compatible Responses
//! API. [`MetaChatDriver`] wraps the shared Responses protocol driver and adds
//! first-party endpoint defaults plus model discovery.
//! It is part of the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! let provider = everruns_meta::provider("meta", "model-api-key");
//! assert_eq!(provider.id().as_str(), "meta");
//! ```

mod driver;

pub use driver::{META_DEFAULT_API_URL, MetaChatDriver, provider, register_driver};
pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
