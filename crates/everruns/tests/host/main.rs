#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
mod dependency_direction;
mod hook_dispatcher;
mod host_services_parity;
#[path = "integration/main.rs"]
mod integration;
#[cfg(feature = "mcp")]
mod mcp_runtime_test;
#[cfg(feature = "openai-agents-api")]
mod openai_agents_api;
#[cfg(feature = "openai-agents-api")]
mod openai_agents_api_lifecycle;
#[cfg(feature = "openai-agents-api")]
mod openai_agents_api_observability;
#[cfg(feature = "openai-agents-api")]
mod openai_agents_api_policy;

#[cfg(feature = "openai-agents-api")]
mod agents_api_support;

#[cfg(feature = "hosting-conformance")]
mod hosting_conformance;
