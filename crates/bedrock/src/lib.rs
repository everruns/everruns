//! AWS Bedrock Runtime provider driver for Everruns.
//!
//! `everruns-bedrock` implements the [`LlmDriver`] contract from `everruns-core`
//! using the AWS Bedrock Runtime `ConverseStream` API.
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
//! use everruns_bedrock::{BedrockLlmDriver, register_driver};
//! use everruns_core::DriverRegistry;
//!
//! let mut registry = DriverRegistry::new();
//! register_driver(&mut registry);
//! ```

mod credential;
mod driver;

pub use driver::{BedrockLlmDriver, register_driver};

pub use everruns_core::llm_driver_registry::{DriverRegistry, LlmDriver};
