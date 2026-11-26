#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![doc = include_str!("../README.md")]

#[cfg(feature = "bashkit")]
pub mod bashkit;
#[cfg(feature = "boxd")]
pub mod boxd;
#[cfg(feature = "brave-search")]
pub mod brave_search;
#[cfg(feature = "browserless")]
pub mod browserless;
#[cfg(feature = "slack-channel")]
pub mod channelkit_bridge;
#[cfg(feature = "cursor")]
pub mod cursor;
#[cfg(feature = "daytona")]
pub mod daytona;
#[cfg(feature = "duckduckgo")]
pub mod duckduckgo;
#[cfg(feature = "e2b")]
pub mod e2b;
#[cfg(feature = "filesystem")]
pub mod filesystem;
#[cfg(feature = "github")]
pub mod github;
#[cfg(feature = "image-generation")]
pub mod image_generation;
#[cfg(feature = "kubernetes-sandbox")]
pub mod kubernetes_sandbox;
#[cfg(feature = "lua")]
pub mod lua;
#[cfg(feature = "modal")]
pub mod modal;
#[cfg(feature = "openai-decisions")]
pub mod openai_decisions;
#[cfg(feature = "openrouter")]
pub mod openrouter;
#[cfg(feature = "parallel")]
pub mod parallel;
#[cfg(feature = "sandbox-guest")]
pub mod sandbox_guest;
#[cfg(feature = "slack-channel")]
pub mod slack_channel;
#[cfg(feature = "typesafe")]
pub mod typesafe;
#[cfg(feature = "web-fetch")]
pub mod web_fetch;
#[cfg(feature = "web-search")]
pub mod web_search;
#[cfg(feature = "webhook-channel")]
pub mod webhook_channel;
#[cfg(any(feature = "e2b", feature = "kubernetes-sandbox"))]
pub mod x11_desktop;

#[cfg(any(feature = "modal", feature = "boxd"))]
mod grpc_transport;

#[cfg(all(test, any(feature = "bashkit", feature = "web-fetch")))]
mod credential_test_support;

/// Standard Webhooks signing and verification.
#[cfg(feature = "event-webhooks")]
pub mod standard_webhooks;

#[cfg(feature = "monty")]
pub mod monty;

#[cfg(any(feature = "bashkit", feature = "monty"))]
mod code_mode_timeline;
