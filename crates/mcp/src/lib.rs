//! Transport-agnostic MCP (Model Context Protocol) client shared by the
//! Everruns runtime, worker, and server.
//!
//! See `specs/runtime-mcp.md` for the design. The crate owns the JSON-RPC
//! client (HTTP, and optional stdio behind the `stdio` feature), credential
//! acquisition (`McpAuthProvider`), result mapping, and tool-call routing
//! (`CompositeToolExecutor`). Wire types and tool-name helpers live in
//! `everruns-core` and are re-used as-is.

pub mod auth;
pub mod client;
pub mod executor;
pub mod http;
pub mod result;
pub mod transport;

#[cfg(feature = "stdio")]
pub mod stdio;

pub use auth::{
    McpAuthProvider, McpAuthRequest, McpCredential, NoAuthProvider, StaticAuthProvider,
};
pub use client::McpClient;
pub use executor::{
    CompositeToolExecutor, McpConnectionResolver, McpExecutor, StaticConnectionResolver,
};
pub use http::{HttpTransport, http_call_tool, http_list_tools, http_send_rpc};
pub use result::{extract_json_from_response, map_tool_call_result};
pub use transport::{McpConnection, McpEndpoint, McpTransport};

#[cfg(feature = "stdio")]
pub use stdio::StdioTransport;
