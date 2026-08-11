//! Host-owned outbound network boundary.
//!
//! `EgressService` is the runtime contract for traffic that leaves an
//! Everruns deployment. Provider drivers, capabilities, toolkit integrations,
//! and system services should depend on this trait instead of constructing
//! transport clients directly.

use crate::network_access::NetworkAccessList;
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::pin::Pin;
use thiserror::Error;

pub type EgressResult<T> = std::result::Result<T, EgressError>;
pub type EgressByteStream = Pin<Box<dyn Stream<Item = EgressResult<Vec<u8>>> + Send>>;

#[derive(Debug, Error)]
pub enum EgressError {
    #[error("Invalid egress request: {0}")]
    InvalidRequest(String),

    #[error("Outbound request blocked by network access policy: {url}")]
    NetworkAccessDenied { url: String },

    #[error("Outbound request signing is not configured")]
    SigningUnavailable,

    #[error("Outbound transport error: {0}")]
    Transport(String),
}

impl EgressError {
    #[doc(hidden)]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }
}

/// Logical owner of an outbound request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressRequestKind {
    Provider,
    Capability,
    Integration,
    SystemEmail,
    UtilityLlm,
    Mcp,
    Other(String),
}

/// Request signing behavior requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressSigning {
    Disabled,
    PlatformDefault,
    Required,
}

fn default_signing() -> EgressSigning {
    EgressSigning::Disabled
}

/// Provider-neutral HTTP request carried over the egress boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressRequest {
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<u8>,
    pub kind: EgressRequestKind,
    #[serde(default = "default_signing")]
    pub signing: EgressSigning,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Whether the concrete egress transport must perform DNS-pinned SSRF validation
    /// after all egress policies pass and before connecting (TM-TOOL-018).
    #[serde(default)]
    pub dns_pinning_required: bool,
    /// Pre-resolved socket addresses for DNS pinning (TM-TOOL-018).
    ///
    /// When set, the concrete transport builds a per-request client pinned to
    /// these addresses via `resolve_to_addrs`, closing the TOCTOU window
    /// between URL validation and the actual TCP connect.  Not serialized —
    /// runtime-only hint owned by the egress boundary.
    #[serde(skip)]
    pub pinned_addrs: Option<(String, Vec<std::net::SocketAddr>)>,
}

impl EgressRequest {
    pub fn new(method: impl Into<String>, url: impl Into<String>, kind: EgressRequestKind) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            headers: BTreeMap::new(),
            body: Vec::new(),
            kind,
            signing: EgressSigning::Disabled,
            network_access: None,
            timeout_ms: None,
            dns_pinning_required: false,
            pinned_addrs: None,
        }
    }

    /// Require the egress boundary to perform DNS-pinned SSRF validation after
    /// all egress policy checks pass and before opening a connection.
    pub fn require_dns_pinning(mut self) -> Self {
        self.dns_pinning_required = true;
        self
    }

    /// Pin the outbound connection to pre-resolved addresses (TM-TOOL-018).
    ///
    /// No-op when `addrs` is empty (e.g. IP-literal URLs where the static
    /// check already validated the address).
    ///
    /// Kept public for callers that resolve-then-check themselves and hand the
    /// pinned addresses in (e.g. the web_fetch fetchkit transport and the MCP
    /// client). New direct call sites should prefer `require_dns_pinning()` so
    /// DNS resolution stays inside the egress boundary, after policy checks.
    pub fn pinned_addrs(
        mut self,
        host: impl Into<String>,
        addrs: Vec<std::net::SocketAddr>,
    ) -> Self {
        if !addrs.is_empty() {
            self.pinned_addrs = Some((host.into(), addrs));
        }
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn signing(mut self, signing: EgressSigning) -> Self {
        self.signing = signing;
        self
    }

    pub fn network_access(mut self, network_access: Option<NetworkAccessList>) -> Self {
        self.network_access = network_access;
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

/// Provider-neutral HTTP response returned by the egress boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub struct EgressStreamResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: EgressByteStream,
}

#[async_trait]
pub trait EgressService: Send + Sync {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse>;

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse>;

    fn name(&self) -> &'static str {
        "EgressService"
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisabledEgressService;

#[async_trait]
impl EgressService for DisabledEgressService {
    async fn send(&self, _request: EgressRequest) -> EgressResult<EgressResponse> {
        Err(EgressError::Transport(
            "outbound egress service is disabled".to_string(),
        ))
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::Transport(
            "outbound egress service is disabled".to_string(),
        ))
    }

    fn name(&self) -> &'static str {
        "DisabledEgressService"
    }
}
