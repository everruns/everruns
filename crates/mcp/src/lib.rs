//! Deprecated compatibility shim for the canonical core module in the
//! [Everruns](https://everruns.com) ecosystem.
//!
//! This is the final forwarding release. Enable the matching `everruns-core`
//! feature and migrate imports to its module before the next platform release.
//!
//! ```
//! use everruns_core::{DisabledEgressService, mcp::{McpClient, NoAuthProvider}};
//! use std::sync::Arc;
//! let client = McpClient::new(Arc::new(DisabledEgressService), Arc::new(NoAuthProvider));
//! let _ = client;
//! ```

#![allow(deprecated)]

#[deprecated(note = "use everruns_core::mcp::CONSENT_TTL")]
pub use everruns_core::mcp::CONSENT_TTL;
#[deprecated(note = "use everruns_core::mcp::CacheHints")]
pub use everruns_core::mcp::CacheHints;
#[deprecated(note = "use everruns_core::mcp::CacheScope")]
pub use everruns_core::mcp::CacheScope;
#[deprecated(note = "use everruns_core::mcp::ClientCapabilities")]
pub use everruns_core::mcp::ClientCapabilities;
#[deprecated(note = "use everruns_core::mcp::ConsentingUrlElicitations")]
pub use everruns_core::mcp::ConsentingUrlElicitations;
#[deprecated(note = "use everruns_core::mcp::DeclineUrlElicitations")]
pub use everruns_core::mcp::DeclineUrlElicitations;
#[deprecated(note = "use everruns_core::mcp::ElicitationAction")]
pub use everruns_core::mcp::ElicitationAction;
#[deprecated(note = "use everruns_core::mcp::ElicitationConsentStore")]
pub use everruns_core::mcp::ElicitationConsentStore;
#[deprecated(note = "use everruns_core::mcp::FORM_ANSWER_TTL")]
pub use everruns_core::mcp::FORM_ANSWER_TTL;
#[deprecated(note = "use everruns_core::mcp::FormAnswer")]
pub use everruns_core::mcp::FormAnswer;
#[deprecated(note = "use everruns_core::mcp::FormAnswerAction")]
pub use everruns_core::mcp::FormAnswerAction;
#[deprecated(note = "use everruns_core::mcp::FormAnswerStore")]
pub use everruns_core::mcp::FormAnswerStore;
#[deprecated(note = "use everruns_core::mcp::FormElicitation")]
pub use everruns_core::mcp::FormElicitation;
#[deprecated(note = "use everruns_core::mcp::FormElicitationHandler")]
pub use everruns_core::mcp::FormElicitationHandler;
#[deprecated(note = "use everruns_core::mcp::FormElicitationPending")]
pub use everruns_core::mcp::FormElicitationPending;
#[deprecated(note = "use everruns_core::mcp::FormOutcome")]
pub use everruns_core::mcp::FormOutcome;
#[deprecated(note = "use everruns_core::mcp::FormRefusal")]
pub use everruns_core::mcp::FormRefusal;
#[deprecated(note = "use everruns_core::mcp::FormSchema")]
pub use everruns_core::mcp::FormSchema;
#[deprecated(note = "use everruns_core::mcp::GrantedConsent")]
pub use everruns_core::mcp::GrantedConsent;
#[deprecated(note = "use everruns_core::mcp::HttpToolsList")]
pub use everruns_core::mcp::HttpToolsList;
#[deprecated(note = "use everruns_core::mcp::HttpTransport")]
pub use everruns_core::mcp::HttpTransport;
#[deprecated(note = "use everruns_core::mcp::MCP_CAPABILITY_PREFIX")]
pub use everruns_core::mcp::MCP_CAPABILITY_PREFIX;
#[deprecated(note = "use everruns_core::mcp::McpAuthProvider")]
pub use everruns_core::mcp::McpAuthProvider;
#[deprecated(note = "use everruns_core::mcp::McpAuthRequest")]
pub use everruns_core::mcp::McpAuthRequest;
#[deprecated(note = "use everruns_core::mcp::McpCapability")]
pub use everruns_core::mcp::McpCapability;
#[deprecated(note = "use everruns_core::mcp::McpCapabilityIdExt")]
pub use everruns_core::mcp::McpCapabilityIdExt;
#[deprecated(note = "use everruns_core::mcp::McpClient")]
pub use everruns_core::mcp::McpClient;
#[deprecated(note = "use everruns_core::mcp::McpConnection")]
pub use everruns_core::mcp::McpConnection;
#[deprecated(note = "use everruns_core::mcp::McpConnectionResolver")]
pub use everruns_core::mcp::McpConnectionResolver;
#[deprecated(note = "use everruns_core::mcp::McpCredential")]
pub use everruns_core::mcp::McpCredential;
#[deprecated(note = "use everruns_core::mcp::McpEndpoint")]
pub use everruns_core::mcp::McpEndpoint;
#[deprecated(note = "use everruns_core::mcp::McpExecutor")]
pub use everruns_core::mcp::McpExecutor;
#[deprecated(note = "use everruns_core::mcp::McpHttpStatusError")]
pub use everruns_core::mcp::McpHttpStatusError;
#[deprecated(note = "use everruns_core::mcp::McpRpcError")]
pub use everruns_core::mcp::McpRpcError;
#[deprecated(note = "use everruns_core::mcp::McpSecretBinding")]
pub use everruns_core::mcp::McpSecretBinding;
#[deprecated(note = "use everruns_core::mcp::McpTransport")]
pub use everruns_core::mcp::McpTransport;
#[deprecated(note = "use everruns_core::mcp::Negotiated")]
pub use everruns_core::mcp::Negotiated;
#[deprecated(note = "use everruns_core::mcp::NoAuthProvider")]
pub use everruns_core::mcp::NoAuthProvider;
#[deprecated(note = "use everruns_core::mcp::RelayUrlElicitations")]
pub use everruns_core::mcp::RelayUrlElicitations;
#[deprecated(note = "use everruns_core::mcp::StaticAuthProvider")]
pub use everruns_core::mcp::StaticAuthProvider;
#[deprecated(note = "use everruns_core::mcp::StaticConnectionResolver")]
pub use everruns_core::mcp::StaticConnectionResolver;
#[cfg(feature = "stdio")]
#[deprecated(note = "use everruns_core::mcp::StdioTransport")]
pub use everruns_core::mcp::StdioTransport;
#[deprecated(note = "use everruns_core::mcp::StoredConsent")]
pub use everruns_core::mcp::StoredConsent;
#[deprecated(note = "use everruns_core::mcp::StoredFormAnswer")]
pub use everruns_core::mcp::StoredFormAnswer;
#[deprecated(note = "use everruns_core::mcp::StoredFormAnswers")]
pub use everruns_core::mcp::StoredFormAnswers;
#[deprecated(note = "use everruns_core::mcp::UrlElicitation")]
pub use everruns_core::mcp::UrlElicitation;
#[deprecated(note = "use everruns_core::mcp::UrlElicitationHandler")]
pub use everruns_core::mcp::UrlElicitationHandler;
#[deprecated(note = "use everruns_core::mcp::UrlElicitationPending")]
pub use everruns_core::mcp::UrlElicitationPending;
#[deprecated(note = "use everruns_core::mcp::consent_storage_key")]
pub use everruns_core::mcp::consent_storage_key;
#[deprecated(note = "use everruns_core::mcp::extract_json_from_response")]
pub use everruns_core::mcp::extract_json_from_response;
#[deprecated(note = "use everruns_core::mcp::form_answer_storage_key")]
pub use everruns_core::mcp::form_answer_storage_key;
#[deprecated(note = "use everruns_core::mcp::http_call_tool")]
pub use everruns_core::mcp::http_call_tool;
#[deprecated(note = "use everruns_core::mcp::http_list_tools")]
pub use everruns_core::mcp::http_list_tools;
#[deprecated(note = "use everruns_core::mcp::http_list_tools_with_cache_hints")]
pub use everruns_core::mcp::http_list_tools_with_cache_hints;
#[deprecated(note = "use everruns_core::mcp::http_request")]
pub use everruns_core::mcp::http_request;
#[deprecated(note = "use everruns_core::mcp::http_send_rpc")]
pub use everruns_core::mcp::http_send_rpc;
#[deprecated(note = "use everruns_core::mcp::is_mcp_capability")]
pub use everruns_core::mcp::is_mcp_capability;
#[deprecated(note = "use everruns_core::mcp::map_tool_call_result")]
pub use everruns_core::mcp::map_tool_call_result;
#[deprecated(note = "use everruns_core::mcp::mcp_capability_id")]
pub use everruns_core::mcp::mcp_capability_id;
#[deprecated(note = "use everruns_core::mcp::parse_mcp_capability_id")]
pub use everruns_core::mcp::parse_mcp_capability_id;
#[deprecated(note = "use everruns_core::mcp::parse_requested_schema")]
pub use everruns_core::mcp::parse_requested_schema;
#[deprecated(note = "use everruns_core::mcp::validate_elicitation_url")]
pub use everruns_core::mcp::validate_elicitation_url;
#[deprecated(note = "use everruns_core::mcp::validate_oauth_resource")]
pub use everruns_core::mcp::validate_oauth_resource;
#[deprecated(note = "use everruns_core::mcp")]
pub use everruns_core::mcp::*;
