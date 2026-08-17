//! AWS Bedrock Runtime provider driver for Everruns.
//!
//! `everruns-bedrock` implements the [`ChatDriver`] contract from `everruns-provider`
//! using the AWS Bedrock Runtime `ConverseStream` API.
//! It is part of the [Everruns](https://everruns.com) ecosystem and pairs with
//! the application-facing `everruns` crate.
//!
//! Credentials are encoded as JSON in the `api_key` field:
//! ```json
//! {"access_key_id":"...","secret_access_key":"...","session_token":"...","region":"us-east-1"}
//! ```
//! `session_token` is optional. The `base_url` field is unused.
//!
//! # Example
//!
//! ```
//! use everruns_bedrock::{BedrockChatDriver, register_driver};
//! use everruns_provider::DriverRegistry;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```

mod credential;
mod driver;

pub use credential::BedrockCredential;
pub use driver::{BedrockAuth, BedrockChatDriver, provider, register_driver};

pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
