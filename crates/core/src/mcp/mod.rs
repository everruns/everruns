#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Transport-agnostic [MCP](https://modelcontextprotocol.io) (Model Context
//! Protocol) client for Everruns agents.
//!
//! Core’s optional `mcp` module is part of the [Everruns](https://everruns.com)
//! ecosystem. Runtime, worker, and server hosts share its client so each host
//! wires MCP without duplicating protocol logic.
//!
//! The module owns the JSON-RPC client (HTTP over injected egress, and optional
//! stdio behind the separate `mcp-stdio` feature), credential acquisition
//! ([`McpAuthProvider`](crate::mcp::McpAuthProvider)), result mapping, and tool execution ([`McpExecutor`](crate::mcp::McpExecutor),
//! which implements
//! `everruns_core::McpToolInvoker` so MCP tools register as regular `Tool`s).
//! Wire types and tool-name helpers live in `everruns-core` and are reused
//! as-is.
//!
//! # Example
//!
//! ```
//! use everruns_core::DisabledEgressService;
//! use everruns_core::mcp::{McpClient, McpConnection, NoAuthProvider};
//! use std::sync::Arc;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let client = McpClient::new(Arc::new(DisabledEgressService), Arc::new(NoAuthProvider));
//! let connection = McpConnection::http("docs", "https://example.com/mcp");
//! # let _ = (client, connection);
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod capability;
pub mod client;
pub mod elicitation;
pub mod executor;
pub mod form_elicitation;
pub mod http;
pub mod oauth;
pub mod protocol;
pub mod result;
pub mod transport;
pub mod user_store;

#[cfg(feature = "mcp-stdio")]
pub mod stdio;

pub use auth::{
    McpAuthProvider, McpAuthRequest, McpCredential, NoAuthProvider, StaticAuthProvider,
};
pub use capability::{
    MCP_CAPABILITY_PREFIX, McpCapability, McpCapabilityIdExt, McpToolLabels, is_mcp_capability,
    mcp_capability_id, parse_mcp_capability_id,
};
pub use client::McpClient;
pub use elicitation::{
    CONSENT_TTL, ConsentingUrlElicitations, DeclineUrlElicitations, ElicitationAction,
    ElicitationConsentStore, GrantedConsent, RelayUrlElicitations, StoredConsent, UrlElicitation,
    UrlElicitationHandler, UrlElicitationPending, consent_storage_key, validate_elicitation_url,
};
pub use executor::{McpConnectionResolver, McpExecutor, StaticConnectionResolver};
pub use form_elicitation::{
    FORM_ANSWER_TTL, FormAnswer, FormAnswerAction, FormAnswerStore, FormElicitation,
    FormElicitationHandler, FormElicitationPending, FormOutcome, FormRefusal, FormSchema,
    StoredFormAnswer, StoredFormAnswers, form_answer_storage_key, parse_requested_schema,
};
pub use http::{
    HttpToolsList, HttpTransport, McpHttpStatusError, McpRpcError, http_call_tool, http_list_tools,
    http_list_tools_with_cache_hints, http_request, http_send_rpc,
};
pub use oauth::validate_oauth_resource;
pub use protocol::{CacheHints, CacheScope, ClientCapabilities, Negotiated};
pub use result::{extract_json_from_response, map_tool_call_result};
pub use transport::{McpConnection, McpEndpoint, McpSecretBinding, McpTransport};
pub use user_store::{
    McpLogin, McpLoginPrompter, USER_MCP_CAPABILITY_ID, USER_MCP_CONNECT_SETTING,
    UserMcpLoginStatus, UserMcpServerEntry, UserMcpServerSummary, UserMcpStore, UserMcpStoreCall,
    UserMcpStoreError, UserMcpStoreReply, UserMcpStoreResult,
};

#[cfg(feature = "mcp-stdio")]
pub use stdio::StdioTransport;
