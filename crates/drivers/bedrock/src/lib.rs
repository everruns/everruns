#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
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
//! With the opt-in `default-credentials` feature, [`BedrockAuth::default_chain`]
//! and [`provider_from_default_chain`] authenticate through the AWS default
//! credential chain instead (environment, profile/SSO, web identity,
//! ECS/AgentCore container credentials, instance profile), so a process running
//! under an IAM role needs no static keys.
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
#[cfg(feature = "default-credentials")]
mod default_chain;
mod driver;

pub use credential::BedrockCredential;
#[cfg(feature = "default-credentials")]
pub use driver::provider_from_default_chain;
pub use driver::{BedrockAuth, BedrockChatDriver, descriptor, from_env, provider, register_driver};

pub use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
