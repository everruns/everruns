#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
mod dependency_direction;
mod hook_dispatcher;
mod host_services_parity;
#[path = "integration/main.rs"]
mod integration;
#[cfg(feature = "mcp")]
mod mcp_catalog_lifecycle;
#[cfg(feature = "mcp")]
mod mcp_runtime_test;
mod tool_catalog_lifecycle;

#[cfg(feature = "hosting-conformance")]
mod hosting_conformance;
