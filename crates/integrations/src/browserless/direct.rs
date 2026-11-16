//! Host-owned local or remote Chromium. This does not inherit Browserless's
//! HTTP relay: the host owns browser provisioning, authentication and network
//! isolation, including subresources and WebSockets. Never expose raw CDP publicly.
//! THREAT[TM-BROWSER-001]: trusted, validated transport; endpoint/auth redacted.
//! THREAT[TM-BROWSER-003]: explicit browser-owned network boundary, not hosted relay.
use std::{collections::HashMap, fmt, sync::Arc};

use everruns_contracts::{
    runtime::browser_use::{BROWSER_USE_SYSTEM_PROMPT, BrowserUseConfig},
    runtime::capabilities::{Capability, CapabilityStatus, RiskLevel},
    runtime::tools::Tool,
    typed_id::SessionId,
};
use serde_json::Value;
use tokio::sync::Mutex;

use super::{browser_use::BrowserTool, cdp::CdpSession};

/// Explicit acknowledgement that the host, not the Browserless relay, owns egress.
#[derive(Clone, Copy, Debug)]
pub enum BrowserNetworkMode {
    BrowserOwned,
}

/// Trusted host configuration, never deserialized from agent tool arguments.
#[derive(Clone)]
pub struct DirectBrowserConfig {
    pub(crate) endpoint: String,
    pub(crate) headers: Vec<(String, String)>,
}

impl fmt::Debug for DirectBrowserConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectBrowserConfig")
            .field("endpoint", &"<redacted>")
            .field("header_count", &self.headers.len())
            .finish()
    }
}

impl DirectBrowserConfig {
    /// Browser-root CDP endpoint. Plain WebSocket is limited to literal loopback;
    /// remote connections require TLS. Userinfo and fragments are refused.
    pub fn new(endpoint: impl Into<String>, _network: BrowserNetworkMode) -> Result<Self, String> {
        let endpoint = endpoint.into();
        let url = url::Url::parse(&endpoint).map_err(|_| "Invalid browser endpoint")?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if !matches!(url.scheme(), "ws" | "wss")
            || (url.scheme() == "ws" && !loopback)
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("Browser endpoint must use wss (or ws on literal loopback), without userinfo or fragment".into());
        }
        Ok(Self {
            endpoint,
            headers: vec![],
        })
    }

    /// Add an authentication header from trusted host configuration. Protocol
    /// handshake headers cannot be overridden. Values are never shown in Debug.
    pub fn header(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, String> {
        use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
        let name = name.into();
        let value = value.into();
        let parsed = name
            .parse::<HeaderName>()
            .map_err(|_| "Invalid browser header name")?;
        value
            .parse::<HeaderValue>()
            .map_err(|_| "Invalid browser header value")?;
        if matches!(parsed.as_str(), "host" | "connection" | "upgrade")
            || parsed.as_str().starts_with("sec-websocket-")
        {
            return Err("Browser handshake headers cannot be overridden".into());
        }
        self.headers.push((name, value));
        Ok(self)
    }
}

pub(crate) struct DirectBackend {
    pub config: DirectBrowserConfig,
    // Opaque context IDs stay host-owned, never accepted from session KV or model output.
    // The lock covers complete actions, so callers outside the act scheduler also serialize.
    pub contexts: Mutex<HashMap<SessionId, Arc<Mutex<Option<String>>>>>,
}

impl DirectBackend {
    pub async fn session(&self, id: SessionId) -> Arc<Mutex<Option<String>>> {
        self.contexts.lock().await.entry(id).or_default().clone()
    }
}

/// One reusable browser capability for Framework and CLI hosts. Clone retains
/// session ownership. Contexts survive tool calls, not a host process restart.
/// Call `close` when the owning session ends. Browser lifetime remains host-owned.
#[derive(Clone)]
pub struct DirectBrowserCapability {
    pub(crate) backend: Arc<DirectBackend>,
}

impl DirectBrowserCapability {
    pub fn new(config: DirectBrowserConfig) -> Self {
        Self {
            backend: Arc::new(DirectBackend {
                config,
                contexts: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Close all contexts owned by this capability after the host has stopped
    /// accepting new work. This includes child-agent sessions. Errors retain
    /// ownership for a later retry; all sessions are attempted.
    pub async fn close_all(&self) -> Result<(), String> {
        let ids: Vec<_> = self.backend.contexts.lock().await.keys().copied().collect();
        let mut failed = false;
        for id in ids {
            failed |= self.close(id).await.is_err();
        }
        if failed {
            Err("Some browser contexts could not be closed".into())
        } else {
            Ok(())
        }
    }

    /// Dispose only the context this capability created for this session.
    /// An unavailable browser returns an error, keeping ownership for a retry.
    pub async fn close(&self, session_id: SessionId) -> Result<(), String> {
        let owned = self.backend.session(session_id).await;
        let mut context = owned.lock().await;
        if let Some(id) = context.as_ref() {
            CdpSession::dispose_direct(
                &self.backend.config.endpoint,
                &self.backend.config.headers,
                id,
            )
            .await?;
            *context = None;
        }
        Ok(())
    }
}

impl Capability for DirectBrowserCapability {
    fn id(&self) -> &str {
        "browser_use"
    }
    fn name(&self) -> &str {
        "Host Browser"
    }
    fn description(&self) -> &str {
        "Operate an isolated local or remote browser. The host owns its network boundary and lifecycle."
    }
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some(BROWSER_USE_SYSTEM_PROMPT)
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }
    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BrowserTool::with_direct(
            BrowserUseConfig::from_value_or_default(config),
            self.backend.clone(),
        ))]
    }
    fn config_schema(&self) -> Option<Value> {
        Some(BrowserUseConfig::json_schema())
    }
    fn validate_config(&self, config: &Value) -> Result<(), String> {
        BrowserUseConfig::from_value(config).map(|_| ())
    }
    fn driver_options(&self, config: &Value) -> Vec<(String, Value)> {
        BrowserUseConfig::from_value_or_default(config).driver_options()
    }
    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }
}
