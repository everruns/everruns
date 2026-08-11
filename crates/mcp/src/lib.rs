//! Transport-agnostic [MCP](https://modelcontextprotocol.io) (Model Context
//! Protocol) client for Everruns agents.
//!
//! `everruns-mcp` is part of the [Everruns](https://everruns.com) ecosystem. It
//! is the shared MCP client used across Everruns hosts (runtime, worker, and
//! server), so every host wires MCP the same way without duplicating protocol
//! logic.
//!
//! The crate owns the JSON-RPC client (HTTP, and optional stdio behind the
//! `stdio` feature), credential acquisition ([`McpAuthProvider`]), result
//! mapping, and tool execution ([`McpExecutor`], which implements
//! `everruns_core::McpToolInvoker` so MCP tools register as regular `Tool`s).
//! Wire types and tool-name helpers live in `everruns-core` and are reused
//! as-is.
//!
//! # Example
//!
//! ```
//! use everruns_mcp::{McpClient, McpConnection};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let client = McpClient::direct();
//! let connection = McpConnection::http("docs", "https://example.com/mcp");
//! let tools = client.discover(&connection).await?;
//! let result = client
//!     .call(&connection, &tools[0].name, serde_json::json!({}))
//!     .await?;
//! # let _ = result;
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod capability;
pub mod client;
pub mod executor;
pub mod http;
pub mod oauth;
pub mod protocol;
pub mod result;
pub mod transport;

#[cfg(feature = "stdio")]
pub mod stdio;

pub use auth::{
    McpAuthProvider, McpAuthRequest, McpCredential, NoAuthProvider, StaticAuthProvider,
};
pub use capability::{
    MCP_CAPABILITY_PREFIX, McpCapability, McpCapabilityIdExt, is_mcp_capability, mcp_capability_id,
    parse_mcp_capability_id,
};
pub use client::McpClient;
pub use executor::{McpConnectionResolver, McpExecutor, StaticConnectionResolver};
pub use http::{HttpTransport, http_call_tool, http_list_tools, http_send_rpc};
pub use protocol::Negotiated;
pub use result::{extract_json_from_response, map_tool_call_result};
pub use transport::{McpConnection, McpEndpoint, McpSecretBinding, McpTransport};

#[cfg(feature = "stdio")]
pub use stdio::StdioTransport;
