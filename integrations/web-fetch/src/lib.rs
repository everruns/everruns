//! FetchKit-backed web fetch capability for Everruns agents.
//!
//! Requests cross the host egress contract, downloads use the session
//! filesystem, and optional bot-auth signs requests with Ed25519 HTTP message
//! signatures. Inline binary responses are rejected.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and is an
//! opt-in network integration for `everruns-host`.
//!
//! # Example
//!
//! ```
//! use everruns_core::Capability;
//! use everruns_integrations_web_fetch::WebFetchCapability;
//!
//! assert_eq!(WebFetchCapability::new(None).id(), "web_fetch");
//! ```

use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use base64::Engine as _;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext,
};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::tool_context::ToolContext;
use everruns_core::*;
#[cfg(test)]
use everruns_provider::error;
use everruns_provider::{tool_types, typed_id};
use fetchkit::file_saver::{FileSaveError, FileSaver, SaveResult};
use fetchkit::{BotAuthConfig, FetchError, FetchRequest};
use serde_json::Value;
use std::result::Result;
use std::sync::Arc;

mod egress_transport;

pub const WEB_FETCH_CAPABILITY_ID: &str = "web_fetch";

/// Agent-facing web-fetch configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WebFetch {
    enable_file_download: bool,
}

impl WebFetch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allow fetched content to be saved into the session workspace.
    pub fn enable_file_download(mut self, enabled: bool) -> Self {
        self.enable_file_download = enabled;
        self
    }
}

impl everruns_capability::IntoCapability for WebFetch {
    fn into_capability(self) -> everruns_capability::CapabilitySpec {
        everruns_capability::CapabilityRef::new(WEB_FETCH_CAPABILITY_ID)
            .config(serde_json::json!({
                "enable_file_download": self.enable_file_download,
            }))
            .into()
    }
}

/// Ed25519 public key JWK derived from a signing key seed.
///
/// Used to register the public key in the HTTP message signatures directory
/// so target servers can verify request signatures.
#[derive(Debug, Clone)]
pub struct BotAuthPublicKey {
    /// JWK Thumbprint (RFC 7638) — matches `BotAuthConfig::keyid()`
    pub key_id: String,
    /// Full JWK object: `{"kty":"OKP","crv":"Ed25519","x":"<base64url>"}`
    pub jwk: serde_json::Value,
}

/// Derive the Ed25519 public key JWK and key ID from a base64url-encoded seed.
///
/// Returns `None` if the seed is invalid. The key_id is the JWK Thumbprint
/// (base64url-encoded SHA-256 of the canonical JWK representation), matching
/// the keyid that fetchkit's `BotAuthConfig` puts in `Signature-Input`.
pub fn derive_bot_auth_public_key(base64_seed: &str) -> Option<BotAuthPublicKey> {
    use base64::Engine as _;
    use ed25519_dalek::SigningKey;
    use sha2::{Digest, Sha256};

    // Decode seed (base64url, no padding)
    let seed_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(base64_seed)
        .ok()?;
    if seed_bytes.len() != 32 {
        return None;
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);

    // Derive public key
    let signing_key = SigningKey::from_bytes(&seed);
    let public_key = signing_key.verifying_key();
    let public_key_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public_key.as_bytes());

    // Build canonical JWK (RFC 7638 member ordering for OKP: crv, kty, x)
    let canonical_jwk = format!(
        r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#,
        public_key_b64
    );

    // JWK Thumbprint = base64url(SHA-256(canonical_jwk))
    let thumbprint = Sha256::digest(canonical_jwk.as_bytes());
    let key_id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(thumbprint);

    let jwk = serde_json::json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "x": public_key_b64,
    });

    Some(BotAuthPublicKey { key_id, jwk })
}

/// WebFetch capability — fetches web content, optionally saves to session filesystem.
///
/// File download is enabled via per-capability config: `{"enable_file_download": true}`.
/// Bot-auth signing is server-wide: set `BOT_AUTH_SIGNING_KEY_SEED` env var.
/// Description, schema, and system prompt all come from fetchkit's ToolBuilder,
/// adapting to whether file download is on.
pub struct WebFetchCapability {
    /// Server-wide bot-auth config (from env vars). When set, all outbound
    /// HTTP requests are signed with Ed25519 per RFC 9421.
    bot_auth: Option<BotAuthConfig>,
}

impl WebFetchCapability {
    /// Create with optional server-wide bot-auth signing config.
    pub fn new(bot_auth: Option<BotAuthConfig>) -> Self {
        Self { bot_auth }
    }

    /// Create from environment variables.
    ///
    /// - `BOT_AUTH_SIGNING_KEY_SEED`: base64url-encoded 32-byte Ed25519 seed (required to enable)
    /// - `BOT_AUTH_AGENT_FQDN`: FQDN for Signature-Agent header (optional)
    /// - `BOT_AUTH_VALIDITY_SECS`: signature validity in seconds (optional, default 300)
    pub fn from_env() -> Self {
        Self {
            bot_auth: bot_auth_config_from_env(),
        }
    }
}

/// Read bot-auth config from environment variables.
fn bot_auth_config_from_env() -> Option<BotAuthConfig> {
    let seed = std::env::var("BOT_AUTH_SIGNING_KEY_SEED").ok()?;

    let mut config = match BotAuthConfig::from_base64_seed(&seed) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "invalid BOT_AUTH_SIGNING_KEY_SEED, bot-auth disabled");
            return None;
        }
    };

    if let Ok(fqdn) = std::env::var("BOT_AUTH_AGENT_FQDN") {
        config = config.with_agent_fqdn(&fqdn);
    }

    if let Ok(secs) = std::env::var("BOT_AUTH_VALIDITY_SECS")
        && let Ok(secs) = secs.parse::<u64>()
    {
        config = config.with_validity_secs(secs);
    }

    tracing::info!("bot-auth request signing enabled");
    Some(config)
}

#[async_trait]
impl Capability for WebFetchCapability {
    fn id(&self) -> &str {
        WEB_FETCH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Web Fetch"
    }

    fn description(&self) -> &str {
        fetchkit::TOOL_DESCRIPTION
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("globe")
    }

    fn category(&self) -> Option<&str> {
        Some("Network")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        None
    }

    fn system_prompt_preview(&self) -> Option<String> {
        // Preview with all features for UI display
        Some(
            fetchkit::Tool::builder()
                .enable_save_to_file(true)
                .build()
                .llmtxt(),
        )
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &serde_json::Value,
    ) -> Option<String> {
        // Behavioral note only — parameter details live in the tool's JSON
        // schema. The full fetchkit llmtxt remains available via
        // `system_prompt_preview()` for UI display but is not injected on
        // every turn. The `save_to_file` mention is gated on the same
        // `enable_file_download` flag the tool itself uses, so the prompt
        // matches the actually-available capability.
        let enable_file_download = config
            .get("enable_file_download")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let body = if enable_file_download {
            "`web_fetch` fetches one URL (GET/HEAD); it is not a search engine. For large or binary responses, pass `save_to_file` to write the body to the workspace instead of inlining it."
        } else {
            "`web_fetch` fetches one URL (GET/HEAD); it is not a search engine."
        };
        Some(format!(
            "<capability id=\"{}\">\n{}\n</capability>",
            self.id(),
            body
        ))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(WebFetchTool::new(false, self.bot_auth.clone()))]
    }

    fn tools_with_config(&self, config: &serde_json::Value) -> Vec<Box<dyn Tool>> {
        let enable_file_download = config
            .get("enable_file_download")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        vec![Box::new(WebFetchTool::new(
            enable_file_download,
            self.bot_auth.clone(),
        ))]
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "enable_file_download": {
                    "type": "boolean",
                    "title": "Allow saving fetched files",
                    "description": "Let the web_fetch tool save large or binary responses \
                                    into the session workspace via save_to_file instead of \
                                    inlining them.",
                    "default": false
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("web_fetch config must be an object".to_string());
        }
        match config.get("enable_file_download") {
            None | Some(serde_json::Value::Bool(_)) => Ok(()),
            Some(other) => Err(format!(
                "enable_file_download must be a boolean, got {other}"
            )),
        }
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Controls whether fetched responses may be saved into the session \
                     workspace.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Отримання вебвмісту"),
                description: Some(
                    "Отримує вміст за URL-адресою (GET/HEAD) і за потреби зберігає його у \
                     файлову систему сесії.",
                ),
                config_description: Some(
                    "Визначає, чи можна зберігати отримані відповіді в робочий простір сесії.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "enable_file_download": {
                            "title": "Дозволити збереження файлів",
                            "description": "Дозволяє інструменту web_fetch зберігати великі або бінарні відповіді у файли робочого простору (save_to_file) замість вбудовування у відповідь."
                        }
                    }
                })),
            },
        ]
    }
}

// ============================================================================
// SessionFileSaver — bridges fetchkit::FileSaver to SessionFileSystem
// ============================================================================

/// Adapter that routes fetchkit file saves through the session virtual filesystem.
///
/// Binary content is encoded as base64; text content is stored as-is.
struct SessionFileSaver {
    file_store: Arc<dyn SessionFileSystem>,
    session_id: SessionId,
}

impl SessionFileSaver {
    async fn resolve_destination(&self, path: &str) -> Result<String, FileSaveError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(FileSaveError::PathNotAllowed(
                "Destination path must name a file".to_string(),
            ));
        }

        // SessionFileSystem is the path authority: this preserves mount routing,
        // agent-facing display identity, and backend containment policy.
        let resolved = self.file_store.resolve_path(path);
        let root = self.file_store.resolve_path("");
        if resolved == root || resolved == "/" {
            return Err(FileSaveError::PathNotAllowed(format!(
                "Destination resolves to the workspace root: {resolved}"
            )));
        }

        let existing = self
            .file_store
            .stat_file(self.session_id, &resolved)
            .await
            .map_err(|error| {
                FileSaveError::Other(format!(
                    "Could not inspect destination path {resolved}: {error}"
                ))
            })?;
        if existing.is_some_and(|entry| entry.is_directory) {
            return Err(FileSaveError::PathNotAllowed(format!(
                "Destination is an existing directory: {resolved}"
            )));
        }

        Ok(resolved)
    }
}

#[async_trait]
impl FileSaver for SessionFileSaver {
    async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError> {
        // Repeat upstream's preflight validation at save time so a changed
        // destination cannot bypass the filesystem's path policy.
        let path = self.resolve_destination(path).await?;
        let (content, encoding) = match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_string(), "text"),
            Err(_) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                (encoded, "base64")
            }
        };

        let file = self
            .file_store
            .write_file(self.session_id, &path, &content, encoding)
            .await
            .map_err(|e| FileSaveError::Other(e.to_string()))?;

        Ok(SaveResult {
            path: file.path,
            bytes_written: bytes.len() as u64,
        })
    }

    async fn validate_path(&self, path: &str) -> Result<(), FileSaveError> {
        self.resolve_destination(path).await.map(|_| ())
    }
}

// ============================================================================
// Tool: web_fetch
// ============================================================================

/// Tool that fetches content from a URL using fetchkit
///
/// THREAT[TM-API-008]: SSRF protection via fetchkit DnsPolicy
/// Mitigation: Default FetchOptions uses DnsPolicy::block_private_ips(),
/// which blocks loopback, RFC1918, link-local (cloud metadata), and other
/// reserved IP ranges via resolve-then-check with DNS pinning.
///
/// File download: when `save_to_file` is provided, content is saved through
/// the session filesystem (SessionFileSystem) via the SessionFileSaver adapter.
pub struct WebFetchTool {
    /// Builder template for this tool's fetchkit configuration. Cloned per
    /// execution to inject the egress transport when the context provides an
    /// `EgressService` (see `egress_transport`).
    builder: fetchkit::ToolBuilder,
    /// Direct (non-egress) tool built from `builder`: serves metadata
    /// (schema/description) and execution for contexts without an egress
    /// service (e.g. embedded hosts), where fetchkit owns the HTTP client.
    fetchkit_tool: fetchkit::Tool,
    enable_save_to_file: bool,
    /// Cached description from ToolBuilder (owned copy of fetchkit's &str for our Tool trait)
    description: String,
    /// Host-wide system allowlist ("green list"), pre-checked on the initial
    /// URL for a clear, distinct system-policy error. On the egress path the
    /// boundary independently re-enforces it (final enforcement point, every
    /// hop); on the direct path this pre-flight is the only enforcement.
    /// `None` = no global enforcement. See `crate::system_allowlist` and
    /// `knowledge/operations/system-allowlist.md`.
    system_allowlist: Option<Arc<crate::system_allowlist::SystemAllowlist>>,
}

impl WebFetchTool {
    /// Create a new WebFetchTool with file download and optional bot-auth signing.
    pub fn new(enable_save_to_file: bool, bot_auth: Option<BotAuthConfig>) -> Self {
        // Decision: in-process JavaScript rendering (fetchkit's `render-rakers`)
        // is not enabled. It linked a JS engine and an HTML/CSS selector stack
        // into every binary for an opt-in fetch mode; JS-rendered pages are
        // served by the browserless and deno integrations instead.
        let mut builder = fetchkit::Tool::builder().enable_save_to_file(enable_save_to_file);
        if let Some(config) = bot_auth {
            builder = builder.bot_auth(config);
        }
        let fetchkit_tool = builder.build();
        let description = fetchkit_tool.description().to_string();
        Self {
            builder,
            fetchkit_tool,
            enable_save_to_file,
            description,
            system_allowlist: crate::system_allowlist::SystemAllowlist::from_env(),
        }
    }

    /// Reject URLs not covered by the active system allowlist with an explicit
    /// system-policy error. Returns `None` when the allowlist is disabled or the
    /// URL is permitted.
    fn system_policy_block(&self, url: &str) -> Option<ToolExecutionResult> {
        match &self.system_allowlist {
            Some(allowlist) if !allowlist.is_url_allowed(url) => {
                Some(ToolExecutionResult::tool_error(format!(
                    "Endpoint blocked by system policy: {url} is not on the allowlist \
                     of permitted public resources."
                )))
            }
            _ => None,
        }
    }

    /// Crawling can issue requests beyond the seed URL. The direct FetchKit
    /// transport cannot apply Everruns URL policy to those discovered pages.
    fn crawl_requires_egress(&self, request: &FetchRequest, context: Option<&ToolContext>) -> bool {
        request.crawl == Some(true)
            && (self.system_allowlist.is_some()
                || context
                    .and_then(|context| context.network_access.as_ref())
                    .is_some_and(|acl| !acl.is_empty()))
            && context.is_none_or(|context| context.egress_service.is_none())
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new(false, None)
    }
}

impl WebFetchTool {
    /// Build a FetchRequest from JSON arguments.
    fn parse_request(arguments: &Value) -> Result<FetchRequest, ToolExecutionResult> {
        let url = match arguments.get("url").and_then(Value::as_str) {
            Some(url) => url.to_string(),
            None => {
                return Err(ToolExecutionResult::tool_error(
                    "Missing required parameter: url",
                ));
            }
        };

        let method = match arguments.get("method") {
            None | Some(Value::Null) => fetchkit::HttpMethod::Get,
            Some(Value::String(method)) if method.eq_ignore_ascii_case("GET") => {
                fetchkit::HttpMethod::Get
            }
            Some(Value::String(method)) if method.eq_ignore_ascii_case("HEAD") => {
                fetchkit::HttpMethod::Head
            }
            _ => {
                return Err(ToolExecutionResult::tool_error(
                    "Invalid method: must be GET or HEAD",
                ));
            }
        };

        // Deserialize the upstream request contract so newly adopted FetchKit
        // fields are forwarded instead of silently discarded by this adapter.
        // Preserve the wrapper's case-insensitive method handling and trim file
        // destinations for callers that bypass JSON Schema validation.
        let mut normalized_arguments = arguments.clone();
        let object = normalized_arguments
            .as_object_mut()
            .ok_or_else(|| ToolExecutionResult::tool_error("Arguments must be a JSON object"))?;
        object.remove("method");
        if let Some(path) = object.get("save_to_file").and_then(Value::as_str) {
            let path = path.trim();
            if path.is_empty() {
                object.remove("save_to_file");
            } else {
                object.insert("save_to_file".to_string(), Value::String(path.to_string()));
            }
        }

        let mut request: FetchRequest =
            serde_json::from_value(normalized_arguments).map_err(|error| {
                ToolExecutionResult::tool_error(format!("Invalid arguments: {error}"))
            })?;
        request.url = url;
        request.method = Some(method);
        Ok(request)
    }

    /// Map a fetchkit error to a ToolExecutionResult.
    fn map_error(e: FetchError) -> ToolExecutionResult {
        let error_message = match e {
            FetchError::MissingUrl => "Missing required parameter: url".to_string(),
            FetchError::InvalidUrlScheme => {
                "Invalid URL: must start with http:// or https://".to_string()
            }
            FetchError::InvalidMethod => "Invalid method: must be GET or HEAD".to_string(),
            FetchError::BlockedUrl => "URL is blocked by policy".to_string(),
            FetchError::ClientBuildError(_) => "Failed to create HTTP client".to_string(),
            FetchError::FirstByteTimeout => {
                "Request timed out: server did not respond within 1 second".to_string()
            }
            FetchError::ConnectError(_) => "Failed to connect to server".to_string(),
            FetchError::RequestError(msg) => format!("Request failed: {msg}"),
            FetchError::FetcherError(msg) => format!("Fetch error: {msg}"),
            FetchError::SaveError(msg) => format!("Failed to save file: {msg}"),
            FetchError::SaverNotAvailable => "File saving not available".to_string(),
            FetchError::RenderNotAvailable => "Rendered fetch backend not available".to_string(),
        };
        ToolExecutionResult::tool_error(error_message)
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_web_fetch(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "web_fetch"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Web Fetch")
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.fetchkit_tool.input_schema()
    }

    fn requires_context(&self) -> bool {
        // Needed for save_to_file (SessionFileSystem access)
        true
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_long_running(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        // Without context, save_to_file is not supported — execute normally
        let request = match Self::parse_request(&arguments) {
            Ok(mut req) => {
                req.save_to_file = None; // Cannot save without context
                req
            }
            Err(e) => return e,
        };

        // Host-wide system allowlist applies even without a session context.
        if let Some(blocked) = self.system_policy_block(&request.url) {
            return blocked;
        }

        if self.crawl_requires_egress(&request, None) {
            return ToolExecutionResult::tool_error(
                "Crawl requires an egress service when network policy is active",
            );
        }

        match self.fetchkit_tool.execute(request).await {
            Ok(response) => {
                ToolExecutionResult::success(serde_json::to_value(&response).unwrap_or_else(
                    |_| serde_json::json!({"error": "Failed to serialize response"}),
                ))
            }
            Err(e) => Self::map_error(e),
        }
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let request = match Self::parse_request(&arguments) {
            Ok(req) => req,
            Err(e) => return e,
        };

        if request.save_to_file.is_some() && !self.enable_save_to_file {
            return ToolExecutionResult::tool_error(
                "File download is disabled for this capability",
            );
        }

        // Host-wide system allowlist, pre-checked on the initial URL for a
        // clear, distinct system-policy error. On the egress path the boundary
        // independently re-enforces it on every hop.
        if let Some(blocked) = self.system_policy_block(&request.url) {
            return blocked;
        }

        // THREAT[TM-AGENT-018]: Enforce network access list. This is the
        // user-facing pre-check; on the egress path the boundary re-checks it
        // on every hop.
        if let Some(ref acl) = context.network_access
            && !acl.is_url_allowed(&request.url)
        {
            return ToolExecutionResult::tool_error(format!(
                "URL blocked by network access policy: {}",
                request.url
            ));
        }

        // THREAT[TM-AGENT-018]: FetchKit's direct transport cannot enforce
        // path-scoped policy on pages discovered after the seed request.
        if self.crawl_requires_egress(&request, Some(context)) {
            return ToolExecutionResult::tool_error(
                "Crawl requires an egress service when network policy is active",
            );
        }

        // Egress-backed path (knowledge/operations/egress.md migration step 3): when the host
        // provides an egress service, inject it as fetchkit's HTTP transport.
        // fetchkit keeps the whole pipeline (specialized fetchers, DNS policy,
        // per-hop redirect validation, bot-auth signing, body caps); every
        // HTTP hop crosses the egress boundary, which enforces the network
        // access list and the system allowlist. Without an egress service
        // (e.g. embedded hosts) fetchkit owns the HTTP client directly.
        let routed_tool;
        let tool = match &context.egress_service {
            Some(egress) => {
                // The system allowlist is enforced again at the egress boundary,
                // but fetchkit resolves redirect targets before invoking the transport.
                // When the allowlist is active, keep redirects on the already
                // preflighted host so disallowed cross-host redirect labels cannot
                // leak via DNS before the boundary denies the request.
                let same_host_redirects_only = self.system_allowlist.is_some();
                routed_tool = self
                    .builder
                    .clone()
                    .same_host_redirects_only_if_set(same_host_redirects_only.then_some(true))
                    .transport(Arc::new(egress_transport::EgressHttpTransport::new(
                        egress.clone(),
                        context.network_access.clone(),
                    )))
                    .build();
                &routed_tool
            }
            None => &self.fetchkit_tool,
        };

        // If no save_to_file, use the simple path (no saver needed)
        if request.save_to_file.is_none() {
            return match tool.execute(request).await {
                Ok(response) => {
                    ToolExecutionResult::success(serde_json::to_value(&response).unwrap_or_else(
                        |_| serde_json::json!({"error": "Failed to serialize response"}),
                    ))
                }
                Err(e) => Self::map_error(e),
            };
        }

        // save_to_file requested — need SessionFileSystem
        let file_store = match &context.file_store {
            Some(store) => store.clone(),
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        let saver = SessionFileSaver {
            file_store,
            session_id: context.session_id,
        };

        match tool.execute_with_saver(request, Some(&saver)).await {
            Ok(response) => {
                ToolExecutionResult::success(serde_json::to_value(&response).unwrap_or_else(
                    |_| serde_json::json!({"error": "Failed to serialize response"}),
                ))
            }
            Err(e) => Self::map_error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::typed_id::SessionId;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create a WebFetchTool with permissive DNS policy for wiremock tests
    /// (wiremock binds to 127.0.0.1 which is blocked by default).
    fn tool_for_wiremock() -> WebFetchTool {
        let builder = fetchkit::Tool::builder()
            .enable_save_to_file(true)
            .block_private_ips(false);
        let fetchkit_tool = builder.build();
        let description = fetchkit_tool.description().to_string();
        WebFetchTool {
            builder,
            fetchkit_tool,
            enable_save_to_file: true,
            description,
            system_allowlist: None,
        }
    }

    #[tokio::test]
    async fn system_allowlist_blocks_with_clear_system_policy_error() {
        use crate::system_allowlist::SystemAllowlist;

        let mut tool = tool_for_wiremock();
        tool.system_allowlist = Some(
            SystemAllowlist::from_toml("[groups.test]\nallowed = [\"allowed.example.com\"]\n")
                .map(Arc::new)
                .unwrap(),
        );

        let result = tool
            .execute(serde_json::json!({ "url": "https://blocked.example.com/path" }))
            .await;

        let message = match result {
            ToolExecutionResult::ToolError(message) => message,
            other => panic!("blocked URL should be a tool error, got: {other:?}"),
        };
        assert!(
            message.contains("blocked by system policy"),
            "error should name the system policy, got: {message}"
        );
        assert!(
            message.contains("blocked.example.com"),
            "error should include the URL, got: {message}"
        );
    }

    #[tokio::test]
    async fn legacy_path_system_policy_error_wins_when_both_policies_deny() {
        use crate::system_allowlist::SystemAllowlist;

        let mut tool = tool_for_wiremock();
        tool.system_allowlist = Some(
            SystemAllowlist::from_toml("[groups.test]\nallowed = [\"allowed.example.com\"]\n")
                .map(Arc::new)
                .unwrap(),
        );
        // No egress service → legacy path; ACL denies the URL too.
        let mut context = ToolContext::new(SessionId::new());
        context.network_access = Some(crate::network_access::NetworkAccessList::allow_only([
            "allowed.example.com",
        ]));

        let result = tool
            .execute_with_context(
                serde_json::json!({ "url": "https://blocked.example.com/x" }),
                &context,
            )
            .await;

        assert!(
            matches!(
                &result,
                ToolExecutionResult::ToolError(msg) if msg.contains("blocked by system policy")
            ),
            "operator-level system policy error should take precedence, got: {result:?}"
        );
    }

    /// In-memory SessionFileSystem for testing file downloads
    struct MockFileStore {
        files: tokio::sync::Mutex<std::collections::HashMap<(SessionId, String), (String, String)>>,
        directories: tokio::sync::Mutex<std::collections::HashSet<(SessionId, String)>>,
    }

    impl MockFileStore {
        fn new() -> Self {
            Self {
                files: tokio::sync::Mutex::new(std::collections::HashMap::new()),
                directories: tokio::sync::Mutex::new(std::collections::HashSet::new()),
            }
        }

        async fn add_directory(&self, session_id: SessionId, path: &str) {
            self.directories
                .lock()
                .await
                .insert((session_id, path.to_string()));
        }

        async fn get_file(&self, session_id: SessionId, path: &str) -> Option<(String, String)> {
            self.files
                .lock()
                .await
                .get(&(session_id, path.to_string()))
                .cloned()
        }
    }

    #[async_trait]
    impl SessionFileSystem for MockFileStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> crate::error::Result<Option<crate::session_file::SessionFile>> {
            let guard = self.files.lock().await;
            if let Some((content, encoding)) = guard.get(&(session_id, path.to_string())) {
                Ok(Some(crate::session_file::SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.uuid(),
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or(path).to_string(),
                    content: Some(content.clone()),
                    encoding: encoding.clone(),
                    size_bytes: content.len() as i64,
                    is_directory: false,
                    is_readonly: false,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> crate::error::Result<crate::session_file::SessionFile> {
            self.files.lock().await.insert(
                (session_id, path.to_string()),
                (content.to_string(), encoding.to_string()),
            );
            Ok(crate::session_file::SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                size_bytes: content.len() as i64,
                is_directory: false,
                is_readonly: false,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> crate::error::Result<bool> {
            Ok(false)
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> crate::error::Result<Vec<crate::session_file::FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> crate::error::Result<Option<crate::session_file::FileStat>> {
            if self
                .directories
                .lock()
                .await
                .contains(&(session_id, path.to_string()))
            {
                return Ok(Some(crate::session_file::FileStat {
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or(path).to_string(),
                    is_directory: true,
                    is_readonly: false,
                    size_bytes: 0,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }));
            }
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> crate::error::Result<Vec<crate::session_file::GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> crate::error::Result<crate::session_file::FileInfo> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn crawl_with_network_policy_requires_egress_service() {
        let tool = WebFetchTool::default();
        let mut context = ToolContext::new(SessionId::new());
        context.network_access = Some(crate::network_access::NetworkAccessList::allow_only([
            "https://example.com/api/",
        ]));

        let result = tool
            .execute_with_context(
                serde_json::json!({
                    "url": "https://example.com/api/index.html",
                    "crawl": true
                }),
                &context,
            )
            .await;

        assert!(matches!(
            result,
            ToolExecutionResult::ToolError(message)
                if message == "Crawl requires an egress service when network policy is active"
        ));
    }

    #[tokio::test]
    async fn test_blank_save_to_file_fetches_inline_without_file_store() {
        let tool = WebFetchTool::new(false, None);
        let mut context = ToolContext::new(SessionId::new());
        context.egress_service = Some(Arc::new(CannedEgress::default()));

        let result = tool
            .execute_with_context(
                serde_json::json!({
                    "url": "http://93.184.216.34/file.txt",
                    "save_to_file": "  \n "
                }),
                &context,
            )
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("blank save_to_file should fetch inline: {result:?}");
        };
        assert_eq!(value["content"], "pong from egress");
        assert!(value.get("saved_path").is_none() || value["saved_path"].is_null());
    }

    #[test]
    fn test_save_error_remains_distinct_from_http_errors() {
        let result = WebFetchTool::map_error(FetchError::SaveError(
            "Path not allowed: Destination is an existing directory: /downloads".to_string(),
        ));
        assert!(matches!(
            result,
            ToolExecutionResult::ToolError(message)
                if message == "Failed to save file: Path not allowed: Destination is an existing directory: /downloads"
        ));
    }

    #[tokio::test]
    async fn test_session_file_saver_rejects_workspace_roots_and_directories() {
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        file_store.add_directory(session_id, "/downloads").await;
        let saver = SessionFileSaver {
            file_store: file_store.clone(),
            session_id,
        };

        for path in ["", "   ", "/", "/workspace", "/workspace/"] {
            let error = saver.validate_path(path).await.unwrap_err();
            assert!(matches!(
                saver.save(path, b"must not write").await,
                Err(FileSaveError::PathNotAllowed(_))
            ));
            assert!(
                matches!(error, FileSaveError::PathNotAllowed(_)),
                "root destination {path:?} should be a path error: {error}"
            );
        }

        assert!(matches!(
            saver.save("/downloads", b"must not write").await,
            Err(FileSaveError::PathNotAllowed(_))
        ));
        assert!(file_store.files.lock().await.is_empty());
        let error = saver.validate_path("/downloads").await.unwrap_err();
        assert!(
            matches!(error, FileSaveError::PathNotAllowed(ref message) if message.contains("directory")),
            "directory destination should be a precise path error: {error}"
        );
    }

    #[tokio::test]
    async fn test_session_file_saver_resolves_and_saves_valid_path() {
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let saver = SessionFileSaver {
            file_store: file_store.clone(),
            session_id,
        };

        saver.validate_path("downloads/file.txt").await.unwrap();
        let saved = saver
            .save("downloads/file.txt", b"downloaded")
            .await
            .unwrap();

        assert_eq!(saved.path, "/downloads/file.txt");
        assert_eq!(saved.bytes_written, 10);
        assert_eq!(
            file_store.get_file(session_id, "/downloads/file.txt").await,
            Some(("downloaded".to_string(), "text".to_string()))
        );
    }

    #[tokio::test]
    async fn test_save_to_file_without_context_strips_save() {
        // When execute() is called (no context), save_to_file should be ignored
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/file.txt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("hello")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/file.txt", mock_server.uri()),
                "save_to_file": "/downloads/file.txt"
            }))
            .await;

        // Should succeed with inline content (save_to_file stripped)
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["content"], "hello");
            assert!(value.get("saved_path").is_none() || value["saved_path"].is_null());
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    // ========================================================================
    // Egress-backed path (knowledge/operations/egress.md migration step 3)
    //
    // URLs use public IP literals so `validate_url_dns_pinned` passes without
    // DNS; the egress mock never performs real network I/O.
    // ========================================================================

    /// Canned egress service: always returns 200 text/plain "pong from egress".
    #[derive(Default)]
    struct CannedEgress {
        requests: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl crate::egress::EgressService for CannedEgress {
        async fn send(
            &self,
            request: crate::egress::EgressRequest,
        ) -> crate::egress::EgressResult<crate::egress::EgressResponse> {
            self.requests.lock().unwrap().push(request.url);
            Ok(crate::egress::EgressResponse {
                status: 200,
                headers: [("content-type".to_string(), "text/plain".to_string())]
                    .into_iter()
                    .collect(),
                body: b"pong from egress".to_vec(),
            })
        }

        async fn send_stream(
            &self,
            request: crate::egress::EgressRequest,
        ) -> crate::egress::EgressResult<crate::egress::EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(crate::egress::EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
            })
        }
    }

    struct RedirectingEgress {
        requests: std::sync::Mutex<Vec<String>>,
    }

    impl RedirectingEgress {
        fn requested_urls(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl crate::egress::EgressService for RedirectingEgress {
        async fn send(
            &self,
            request: crate::egress::EgressRequest,
        ) -> crate::egress::EgressResult<crate::egress::EgressResponse> {
            let url = request.url.clone();
            self.requests.lock().unwrap().push(request.url);
            // The final host returns 200; only the initial host issues the
            // cross-host redirect. Returning 200 on the final URL keeps the test
            // deterministic — if the same-host-only policy ever regressed, the
            // second hop would terminate here instead of self-redirecting.
            if url == "http://93.184.216.35/final" {
                return Ok(crate::egress::EgressResponse {
                    status: 200,
                    headers: [("content-type".to_string(), "text/plain".to_string())]
                        .into_iter()
                        .collect(),
                    body: b"final".to_vec(),
                });
            }
            Ok(crate::egress::EgressResponse {
                status: 302,
                headers: [(
                    "location".to_string(),
                    "http://93.184.216.35/final".to_string(),
                )]
                .into_iter()
                .collect(),
                body: Vec::new(),
            })
        }

        async fn send_stream(
            &self,
            request: crate::egress::EgressRequest,
        ) -> crate::egress::EgressResult<crate::egress::EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(crate::egress::EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
            })
        }
    }

    #[tokio::test]
    async fn test_execute_with_context_routes_through_egress() {
        let tool = WebFetchTool::default();
        let mut context = ToolContext::new(SessionId::new());
        let egress = Arc::new(CannedEgress::default());
        context.egress_service = Some(egress.clone());

        let result = tool
            .execute_with_context(
                serde_json::json!({ "url": "http://93.184.216.34/ping" }),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["content"], "pong from egress");
        } else {
            panic!("Expected successful egress-path response, got: {result:?}");
        }
        assert_eq!(
            *egress.requests.lock().unwrap(),
            vec!["http://93.184.216.34/ping".to_string()]
        );
    }

    #[tokio::test]
    async fn test_egress_path_system_allowlist_blocks_cross_host_redirect_before_second_hop() {
        use crate::system_allowlist::SystemAllowlist;

        let tool = WebFetchTool {
            system_allowlist: Some(
                SystemAllowlist::from_toml("[groups.test]\nallowed = [\"93.184.216.34\"]\n")
                    .map(Arc::new)
                    .unwrap(),
            ),
            ..Default::default()
        };
        let egress = Arc::new(RedirectingEgress {
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let mut context = ToolContext::new(SessionId::new());
        context.egress_service = Some(egress.clone());

        let result = tool
            .execute_with_context(
                serde_json::json!({ "url": "http://93.184.216.34/start" }),
                &context,
            )
            .await;

        assert!(
            matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("blocked")),
            "expected cross-host redirect denial, got: {result:?}"
        );
        assert_eq!(
            egress.requested_urls(),
            vec!["http://93.184.216.34/start"],
            "redirect target must be rejected before a second egress hop can resolve it"
        );
    }

    #[tokio::test]
    async fn test_egress_path_denies_url_outside_network_access_list() {
        let tool = WebFetchTool::default();
        let mut context = ToolContext::new(SessionId::new());
        let egress = Arc::new(CannedEgress::default());
        context.egress_service = Some(egress.clone());
        context.network_access = Some(crate::network_access::NetworkAccessList::allow_only([
            "allowed.example.com",
        ]));

        let result = tool
            .execute_with_context(
                serde_json::json!({ "url": "http://93.184.216.34/ping" }),
                &context,
            )
            .await;

        assert!(
            matches!(
                &result,
                ToolExecutionResult::ToolError(msg) if msg.contains("blocked by network access policy")
            ),
            "expected network access denial, got: {result:?}"
        );
        assert_eq!(*egress.requests.lock().unwrap(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn test_egress_path_blocks_private_address_before_sending() {
        let tool = WebFetchTool::default();
        let mut context = ToolContext::new(SessionId::new());
        let egress = Arc::new(CannedEgress::default());
        context.egress_service = Some(egress.clone());

        let result = tool
            .execute_with_context(
                serde_json::json!({ "url": "http://169.254.169.254/latest/meta-data/" }),
                &context,
            )
            .await;

        assert!(
            matches!(
                &result,
                ToolExecutionResult::ToolError(msg) if msg.contains("blocked")
            ),
            "expected SSRF block on egress path, got: {result:?}"
        );
        assert_eq!(*egress.requests.lock().unwrap(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn test_egress_path_save_to_file_writes_session_file() {
        let tool = WebFetchTool::new(true, None);
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let mut context = ToolContext::with_file_store(session_id, file_store.clone());
        let egress = Arc::new(CannedEgress::default());
        context.egress_service = Some(egress.clone());

        let result = tool
            .execute_with_context(
                serde_json::json!({
                    "url": "http://93.184.216.34/file.txt",
                    "save_to_file": "/downloads/file.txt"
                }),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["saved_path"], "/downloads/file.txt");
            assert_eq!(value["bytes_written"], 16);
            assert!(value.get("content").is_none());
            let (content, encoding) = file_store
                .get_file(session_id, "/downloads/file.txt")
                .await
                .expect("file should have been written via egress path");
            assert_eq!(encoding, "text");
            assert_eq!(content, "pong from egress");
        } else {
            panic!("Expected successful response, got: {result:?}");
        }
        assert_eq!(
            *egress.requests.lock().unwrap(),
            vec!["http://93.184.216.34/file.txt".to_string()]
        );
    }
    fn successful(result: ToolExecutionResult) -> Value {
        match result {
            ToolExecutionResult::Success(value) => value,
            other => panic!("expected a successful HTTP response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_conversions_preserve_complete_content_and_response_metadata() {
        let server = MockServer::builder().start().await;
        let html = "<html><body><h1>Title</h1><p>Body</p></body></html>";
        let cases = [
            (
                "/",
                html,
                "text/html; charset=utf-8",
                serde_json::json!({}),
                "raw",
                html,
            ),
            (
                "/nested/document.html",
                html,
                "text/html",
                serde_json::json!({"as_text":true}),
                "text",
                "Title\n\nBody",
            ),
            (
                "/repository/readme",
                html,
                "text/html",
                serde_json::json!({"as_markdown":true}),
                "markdown",
                "# Title\n\nBody",
            ),
            (
                "/data.json",
                "{\"key\":\"value\"}",
                "application/json",
                serde_json::json!({}),
                "raw",
                "{\"key\":\"value\"}",
            ),
            (
                "/newlines",
                "line1\n\n\n\n\nline2",
                "text/plain",
                serde_json::json!({}),
                "raw",
                "line1\n\nline2",
            ),
        ];
        let tool = tool_for_wiremock();
        for (route, body, content_type, mut arguments, format, content) in cases {
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(body, content_type)
                        .insert_header("etag", "\"fixture-v1\"")
                        .insert_header("last-modified", "Wed, 15 Jul 2026 12:00:00 GMT"),
                )
                .expect(1)
                .mount(&server)
                .await;
            let url = format!("{}{route}", server.uri());
            arguments["url"] = serde_json::json!(url);
            let value = successful(tool.execute(arguments).await);
            assert_eq!(value["url"], url);
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["content_type"], content_type);
            assert_eq!(value["size"], body.len());
            assert_eq!(value["format"], format);
            assert_eq!(value["content"], content, "route {route}");
            assert_eq!(value["etag"], "\"fixture-v1\"");
            assert_eq!(value["last_modified"], "Wed, 15 Jul 2026 12:00:00 GMT");
            for absent in [
                "truncated",
                "method",
                "error",
                "saved_path",
                "bytes_written",
            ] {
                assert!(value.get(absent).is_none(), "unexpected {absent}: {value}");
            }
        }
    }

    #[tokio::test]
    async fn head_and_binary_responses_preserve_metadata_without_inline_content() {
        let server = MockServer::builder().start().await;
        let tool = tool_for_wiremock();
        for (verb, route, content_type, size) in [
            ("HEAD", "/head", "text/html", 5000),
            ("GET", "/image.png", "image/png", 4),
        ] {
            let response = ResponseTemplate::new(200)
                .insert_header("content-type", content_type)
                .insert_header("content-length", size.to_string())
                .insert_header("etag", "\"v2\"");
            let response = if verb == "GET" {
                response.set_body_bytes(vec![0x89, 0x50, 0x4e, 0x47])
            } else {
                response
            };
            Mock::given(method(verb))
                .and(path(route))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            let url = format!("{}{route}", server.uri());
            let value = successful(
                tool.execute(serde_json::json!({"url":url,"method":verb}))
                    .await,
            );
            assert_eq!(value["url"], url);
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["content_type"], content_type);
            assert_eq!(value["size"], size);
            assert_eq!(value["etag"], "\"v2\"");
            for absent in ["content", "format", "saved_path", "bytes_written"] {
                assert!(value.get(absent).is_none(), "unexpected {absent}: {value}");
            }
            if verb == "HEAD" {
                assert_eq!(value["method"], "HEAD");
                assert!(value.get("error").is_none());
            } else {
                assert!(value.get("method").is_none());
                assert_eq!(
                    value["error"],
                    "Binary content is not supported. Only textual content (HTML, text, JSON, etc.) can be fetched."
                );
            }
        }
    }

    #[tokio::test]
    async fn http_error_statuses_preserve_the_response_body() {
        let server = MockServer::builder().start().await;
        let tool = tool_for_wiremock();
        for (status, body) in [(404, "Not Found"), (500, "Internal Server Error")] {
            let route = format!("/status/{status}");
            Mock::given(method("GET"))
                .and(path(&route))
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string(body)
                        .insert_header("content-type", "text/plain"),
                )
                .expect(1)
                .mount(&server)
                .await;
            let url = format!("{}{route}", server.uri());
            let value = successful(tool.execute(serde_json::json!({"url":url})).await);
            assert_eq!(value["url"], url);
            assert_eq!(value["status_code"], status);
            assert_eq!(value["content"], body);
            assert_eq!(value["size"], body.len());
            assert_eq!(value["format"], "raw");
        }
    }

    struct FailedTransport {
        kind: u8,
        requests: std::sync::Mutex<Vec<String>>,
    }
    #[async_trait]
    impl fetchkit::HttpTransport for FailedTransport {
        async fn execute(
            &self,
            request: fetchkit::TransportRequest,
        ) -> Result<fetchkit::TransportResponse, fetchkit::TransportError> {
            self.requests.lock().unwrap().push(request.url.to_string());
            Err(match self.kind {
                0 => fetchkit::TransportError::Timeout,
                1 => fetchkit::TransportError::Connect,
                2 => fetchkit::TransportError::Request("connection reset".to_string()),
                _ => fetchkit::TransportError::Other("transport unavailable".to_string()),
            })
        }
    }

    #[tokio::test]
    async fn transport_failures_are_errors_on_both_execution_paths() {
        // Exercise the complete tool/FetchKit/error mapping without DNS or wall-clock waits.
        let url = "http://93.184.216.34/failure";
        for (kind, expected) in [
            (
                0,
                "Request timed out: server did not respond within 1 second",
            ),
            (1, "Request failed: failed to connect to server"),
            (2, "Request failed: connection reset"),
            (3, "Request failed: transport unavailable"),
        ] {
            let transport = Arc::new(FailedTransport {
                kind,
                requests: std::sync::Mutex::new(Vec::new()),
            });
            let builder = fetchkit::Tool::builder().transport(transport.clone());
            let tool = WebFetchTool {
                fetchkit_tool: builder.build(),
                builder,
                description: String::new(),
                enable_save_to_file: false,
                system_allowlist: None,
            };
            for with_context in [false, true] {
                let args = serde_json::json!({"url":url});
                let result = if with_context {
                    tool.execute_with_context(args, &ToolContext::new(SessionId::new()))
                        .await
                } else {
                    tool.execute(args).await
                };
                assert!(
                    matches!(&result,ToolExecutionResult::ToolError(message) if message==expected),
                    "kind {kind}, context {with_context}: {result:?}"
                );
            }
            assert_eq!(
                *transport.requests.lock().unwrap(),
                vec![url.to_string(), url.to_string()]
            );
        }
    }
    #[tokio::test]
    async fn invalid_arguments_and_schemes_fail_before_network_io() {
        let server = MockServer::builder().start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("unexpected request"))
            .expect(0)
            .mount(&server)
            .await;
        let tool = tool_for_wiremock();
        let mut cases = vec![
            (serde_json::json!({}), "Missing required parameter: url"),
            (
                serde_json::json!({"url":false}),
                "Missing required parameter: url",
            ),
            (
                serde_json::json!({"url":"not-a-valid-url"}),
                "Invalid URL: must start with http:// or https://",
            ),
        ];
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/file.txt",
            "gopher://internal-server/",
        ] {
            cases.push((
                serde_json::json!({"url":url}),
                "Invalid URL: must start with http:// or https://",
            ));
        }
        for method in [
            serde_json::json!("POST"),
            serde_json::json!(""),
            serde_json::json!(true),
            serde_json::json!(7),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            cases.push((
                serde_json::json!({"url":server.uri(),"method":method}),
                "Invalid method: must be GET or HEAD",
            ));
        }
        for (args, expected) in cases {
            for with_context in [false, true] {
                let result = if with_context {
                    tool.execute_with_context(args.clone(), &ToolContext::new(SessionId::new()))
                        .await
                } else {
                    tool.execute(args.clone()).await
                };
                assert!(
                    matches!(&result,ToolExecutionResult::ToolError(message) if message==expected),
                    "arguments {args}, context {with_context}: {result:?}"
                );
            }
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn request_normalization_preserves_all_supported_fields() {
        let input = serde_json::json!({
            "url":"https://example.com/docs?a=1","method":"hEaD","as_markdown":true,"as_text":false,
            "save_to_file":"  /downloads/file.txt \n", "content_focus":"agent","crawl":true,"max_pages":3,
            "if_none_match":"\"abc123\"","if_modified_since":"Wed, 15 Jul 2026 12:00:00 GMT"
        });
        let expected = serde_json::json!({
            "url":"https://example.com/docs?a=1","method":"HEAD","as_markdown":true,"as_text":false,
            "save_to_file":"/downloads/file.txt", "content_focus":"agent","crawl":true,"max_pages":3,
            "if_none_match":"\"abc123\"","if_modified_since":"Wed, 15 Jul 2026 12:00:00 GMT"
        });
        assert_eq!(
            serde_json::to_value(WebFetchTool::parse_request(&input).unwrap()).unwrap(),
            expected
        );
        for arguments in [
            serde_json::json!({"url":"https://example.com"}),
            serde_json::json!({"url":"https://example.com","method":null}),
            serde_json::json!({"url":"https://example.com","method":"get","save_to_file":" \n\t "}),
        ] {
            assert_eq!(
                serde_json::to_value(WebFetchTool::parse_request(&arguments).unwrap()).unwrap(),
                serde_json::json!({"url":"https://example.com","method":"GET"})
            );
        }
    }

    #[tokio::test]
    async fn private_address_families_are_blocked_on_both_execution_paths() {
        // Literal addresses do not require external DNS. A policy failure must
        // never be accepted as a connection failure or successful empty response.
        let tool = WebFetchTool::default();
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:1/",
            "http://10.0.0.1:1/",
            "http://172.16.0.1:1/",
            "http://192.168.0.1:1/",
            "http://[::1]:1/",
            "http://0.0.0.0:1/",
        ] {
            for with_context in [false, true] {
                let args = serde_json::json!({"url":url});
                let result = if with_context {
                    tool.execute_with_context(args, &ToolContext::new(SessionId::new()))
                        .await
                } else {
                    tool.execute(args).await
                };
                assert!(
                    matches!(&result,ToolExecutionResult::ToolError(message) if message=="URL is blocked by policy"),
                    "{url}, context {with_context}: {result:?}"
                );
            }
        }
    }
    #[tokio::test]
    async fn downloads_store_exact_bytes_only_in_the_requested_session() {
        let server = MockServer::builder().start().await;
        let tool = tool_for_wiremock();
        let store = Arc::new(MockFileStore::new());
        let session = SessionId::new();
        let other_session = SessionId::new();
        let context = ToolContext::with_file_store(session, store.clone());
        for (route, content_type, bytes, encoding) in [
            (
                "/data.json",
                "application/json",
                b"{\"key\":\"value\"}".to_vec(),
                "text",
            ),
            (
                "/image.png",
                "image/png",
                vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0xfe],
                "base64",
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(200).set_body_raw(bytes.clone(), content_type))
                .expect(1)
                .mount(&server)
                .await;
            let destination = format!("/downloads{route}");
            let url = format!("{}{route}", server.uri());
            let value = successful(
                tool.execute_with_context(
                    serde_json::json!({"url":url,"save_to_file":destination}),
                    &context,
                )
                .await,
            );
            assert_eq!(value["url"], url);
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["saved_path"], destination);
            assert_eq!(value["bytes_written"], bytes.len());
            assert!(value.get("content").is_none());
            assert!(value.get("error").is_none());
            let (stored, stored_encoding) = store
                .get_file(session, &destination)
                .await
                .expect("saved file");
            assert_eq!(stored_encoding, encoding);
            let actual = if encoding == "base64" {
                base64::engine::general_purpose::STANDARD
                    .decode(stored)
                    .unwrap()
            } else {
                stored.into_bytes()
            };
            assert_eq!(actual, bytes);
            assert!(store.get_file(other_session, &destination).await.is_none());
        }
        assert_eq!(store.files.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn download_gates_reject_before_network_or_file_writes() {
        let server = MockServer::builder().start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("must not fetch"))
            .expect(0)
            .mount(&server)
            .await;
        let store = Arc::new(MockFileStore::new());
        let session = SessionId::new();
        for (enabled, with_store, expected) in [
            (false, true, "File download is disabled for this capability"),
            (true, false, "File system not available in this context"),
        ] {
            let mut tool = tool_for_wiremock();
            tool.enable_save_to_file = enabled;
            let context = if with_store {
                ToolContext::with_file_store(session, store.clone())
            } else {
                ToolContext::new(session)
            };
            let result = tool
                .execute_with_context(
                    serde_json::json!({"url":server.uri(),"save_to_file":"/downloads/file.txt"}),
                    &context,
                )
                .await;
            assert!(
                matches!(&result,ToolExecutionResult::ToolError(message) if message==expected),
                "{result:?}"
            );
        }
        store.add_directory(session, "/downloads").await;
        let context = ToolContext::with_file_store(session, store.clone());
        for (destination, diagnostic) in [
            ("/", "workspace root"),
            ("/workspace", "workspace root"),
            ("/downloads", "existing directory"),
        ] {
            let result = tool_for_wiremock()
                .execute_with_context(
                    serde_json::json!({"url":server.uri(),"save_to_file":destination}),
                    &context,
                )
                .await;
            assert!(
                matches!(&result, ToolExecutionResult::ToolError(message) if message.starts_with("Failed to save file:") && message.contains(diagnostic)),
                "{destination}: {result:?}"
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(store.files.lock().await.is_empty());
    }
    #[test]
    fn bot_auth_jwk_matches_the_known_ed25519_vector_and_fetchkit_identity() {
        // RFC 8032 section 7.1, test 1; thumbprint uses RFC 7638 canonical JWK.
        let seed = "nWGxne_9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A";
        let key = derive_bot_auth_public_key(seed).unwrap();
        assert_eq!(
            key.jwk,
            serde_json::json!({"kty":"OKP","crv":"Ed25519","x":"11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"})
        );
        assert_eq!(key.key_id, "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k");
        assert_eq!(
            key.key_id,
            BotAuthConfig::from_base64_seed(seed).unwrap().keyid()
        );
        for invalid in [
            "!!!invalid!!!".to_string(),
            "tooshort".to_string(),
            format!("{seed}="),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 31]),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 33]),
        ] {
            assert!(derive_bot_auth_public_key(&invalid).is_none(), "{invalid}");
        }
    }

    #[tokio::test]
    async fn capability_configuration_aligns_schema_prompt_and_download_behavior() {
        let capability = WebFetchCapability::new(None);
        assert_eq!(capability.id(), "web_fetch");
        assert_eq!(capability.risk_level(), RiskLevel::High);
        assert!(capability.system_prompt_addition().is_none());
        assert!(
            capability
                .system_prompt_preview()
                .unwrap()
                .contains("save_to_file")
        );
        let config_schema = capability.config_schema().unwrap();
        assert_eq!(
            config_schema["properties"]["enable_file_download"]["type"],
            "boolean"
        );
        assert_eq!(
            config_schema["properties"]["enable_file_download"]["default"],
            false
        );
        for invalid in [
            serde_json::json!([]),
            serde_json::json!(true),
            serde_json::json!("true"),
            serde_json::json!({"enable_file_download":null}),
            serde_json::json!({"enable_file_download":"true"}),
        ] {
            assert!(capability.validate_config(&invalid).is_err(), "{invalid}");
        }
        for (config, enabled) in [
            (serde_json::Value::Null, false),
            (serde_json::json!({}), false),
            (serde_json::json!({"enable_file_download":false}), false),
            (serde_json::json!({"enable_file_download":true}), true),
        ] {
            capability.validate_config(&config).unwrap();
            let tools = capability.tools_with_config(&config);
            assert_eq!(tools.len(), 1);
            let tool = &tools[0];
            assert_eq!(tool.name(), "web_fetch");
            assert!(tool.requires_context());
            let schema = tool.parameters_schema();
            assert_eq!(schema["type"], "object");
            assert_eq!(schema["required"], serde_json::json!(["url"]));
            assert_eq!(
                schema["properties"]["method"],
                serde_json::json!({"type":"string","enum":["GET","HEAD"],"default":"GET"})
            );
            for (name, kind) in [
                ("url", "string"),
                ("as_markdown", "boolean"),
                ("as_text", "boolean"),
                ("content_focus", "string"),
                ("crawl", "boolean"),
                ("max_pages", "integer"),
            ] {
                assert_eq!(schema["properties"][name]["type"], kind, "{name}");
            }
            assert_eq!(schema["properties"].get("save_to_file").is_some(), enabled);
            assert!(schema["properties"].get("render").is_none());
            if !enabled {
                let defaults = capability.tools();
                assert_eq!(defaults.len(), 1);
                assert_eq!(defaults[0].parameters_schema(), schema);
            }
            let session = SessionId::new();
            let prompt = capability
                .system_prompt_contribution_with_config(
                    &SystemPromptContext::without_file_store(session),
                    &config,
                )
                .await
                .unwrap();
            assert_eq!(prompt.contains("save_to_file"), enabled);
            assert!(prompt.len() <= if enabled { 350 } else { 250 });
            assert!(prompt.contains("GET/HEAD"));
            assert!(prompt.contains("not a search engine"));
            let store = Arc::new(MockFileStore::new());
            let mut context = ToolContext::with_file_store(session, store.clone());
            context.egress_service = Some(Arc::new(CannedEgress::default()));
            let result=tool.execute_with_context(serde_json::json!({"url":"http://93.184.216.34/config","save_to_file":"/config.txt"}),&context).await;
            if enabled {
                assert_eq!(successful(result)["saved_path"], "/config.txt");
                assert_eq!(
                    store.get_file(session, "/config.txt").await,
                    Some(("pong from egress".to_string(), "text".to_string()))
                );
            } else {
                assert!(
                    matches!(result,ToolExecutionResult::ToolError(message) if message=="File download is disabled for this capability")
                );
                assert!(store.files.lock().await.is_empty());
            }
        }
    }
}
