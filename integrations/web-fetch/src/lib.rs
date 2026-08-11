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
use crate::traits::{SessionFileSystem, ToolContext};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use base64::Engine as _;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext,
};
use everruns_core::*;
use fetchkit::file_saver::{FileSaveError, FileSaver, SaveResult};
use fetchkit::{BotAuthConfig, FetchError, FetchRequest};
use serde_json::Value;
use std::result::Result;
use std::sync::Arc;

mod egress_transport;

pub const WEB_FETCH_CAPABILITY_ID: &str = "web_fetch";

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
                .enable_render_rakers(true)
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
        // Revisit after upgrading fetchkit beyond 0.4.1: confirm upstream calls
        // validate_path before network I/O. Keep this save-time check as defense
        // in depth unless the FileSaver contract guarantees the path is unchanged.
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
        // THREAT[TM-TOOL-024]: Rendering stays request-opt-in; FetchKit runs
        // inline scripts with a timeout, denies renderer subresource traffic,
        // and caps rendered output before conversion.
        let mut builder = fetchkit::Tool::builder()
            .enable_save_to_file(enable_save_to_file)
            .enable_render_rakers(true);
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

        let method = arguments
            .get("method")
            .and_then(|v| v.as_str())
            .map(|s| match s.to_uppercase().as_str() {
                "GET" => Some(fetchkit::HttpMethod::Get),
                "HEAD" => Some(fetchkit::HttpMethod::Head),
                _ => None,
            })
            .unwrap_or(Some(fetchkit::HttpMethod::Get));

        let method = match method {
            Some(m) => m,
            None => {
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

    #[tokio::test]
    async fn system_prompt_variants_stay_within_budget() {
        let cap = WebFetchCapability::new(None);
        let ctx = SystemPromptContext::without_file_store(SessionId::new());

        let disabled = cap
            .system_prompt_contribution_with_config(&ctx, &serde_json::json!({}))
            .await
            .expect("web fetch contributes a prompt");
        assert!(
            disabled.len() <= 250,
            "web fetch prompt without downloads grew to {} bytes",
            disabled.len()
        );

        let enabled = cap
            .system_prompt_contribution_with_config(
                &ctx,
                &serde_json::json!({"enable_file_download": true}),
            )
            .await
            .expect("web fetch contributes a download prompt");
        assert!(
            enabled.len() <= 350,
            "web fetch download prompt grew to {} bytes",
            enabled.len()
        );
    }
    use crate::typed_id::SessionId;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create a WebFetchTool with permissive DNS policy for wiremock tests
    /// (wiremock binds to 127.0.0.1 which is blocked by default).
    fn tool_for_wiremock() -> WebFetchTool {
        let builder = fetchkit::Tool::builder()
            .enable_save_to_file(true)
            .enable_render_rakers(true)
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

    #[test]
    fn test_derive_bot_auth_public_key() {
        // 32 bytes of 'A' (0x41), base64url-encoded
        let seed = "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE";
        let pk = super::derive_bot_auth_public_key(seed).unwrap();

        // JWK has correct structure
        assert_eq!(pk.jwk["kty"], "OKP");
        assert_eq!(pk.jwk["crv"], "Ed25519");
        assert!(pk.jwk["x"].is_string());

        // key_id matches fetchkit's BotAuthConfig::keyid()
        let fetchkit_config = fetchkit::BotAuthConfig::from_base64_seed(seed).unwrap();
        assert_eq!(pk.key_id, fetchkit_config.keyid());
    }

    #[test]
    fn test_derive_bot_auth_public_key_invalid_seed() {
        assert!(super::derive_bot_auth_public_key("tooshort").is_none());
        assert!(super::derive_bot_auth_public_key("!!!invalid!!!").is_none());
    }

    #[test]
    fn test_web_fetch_tool_parameters() {
        let tool = WebFetchTool::default();
        let schema = tool.parameters_schema();

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["url"].is_object());
        assert!(schema["properties"]["method"].is_object());
        assert!(schema["properties"]["as_markdown"].is_object());
        assert!(schema["properties"]["as_text"].is_object());
        assert!(schema["properties"]["content_focus"].is_object());
        assert!(schema["properties"]["crawl"].is_object());
        assert!(schema["properties"]["max_pages"].is_object());
        assert!(schema["properties"]["render"].is_object());
        assert_eq!(schema["required"], serde_json::json!(["url"]));
    }

    #[test]
    fn test_web_fetch_capability_metadata() {
        let cap = WebFetchCapability::new(None);

        assert_eq!(cap.id(), "web_fetch");
        assert_eq!(cap.name(), "Web Fetch");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.risk_level(), RiskLevel::High);
        assert_eq!(cap.icon(), Some("globe"));
        assert_eq!(cap.category(), Some("Network"));
        // System prompt comes from fetchkit ToolBuilder via system_prompt_contribution_with_config
        assert!(cap.system_prompt_addition().is_none());
        // Preview shows full features for UI
        let preview = cap.system_prompt_preview().unwrap();
        assert!(preview.contains("web_fetch"));
    }

    #[test]
    fn test_web_fetch_capability_has_tool() {
        let cap = WebFetchCapability::new(None);
        let tools = cap.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "web_fetch");
    }

    #[tokio::test]
    async fn test_web_fetch_missing_url() {
        let tool = WebFetchTool::default();
        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("url"));
        } else {
            panic!("Expected tool error for missing URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url() {
        let tool = WebFetchTool::default();
        let result = tool
            .execute(serde_json::json!({"url": "not-a-valid-url"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid URL"));
        } else {
            panic!("Expected tool error for invalid URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_method() {
        let tool = WebFetchTool::default();
        let result = tool
            .execute(serde_json::json!({"url": "https://example.com", "method": "POST"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid method"));
        } else {
            panic!("Expected tool error for invalid method");
        }
    }

    // Integration tests using wiremock
    #[tokio::test]
    async fn test_web_fetch_real_request() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body><p>Herman Melville - Moby Dick</p></body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(
                value["content"]
                    .as_str()
                    .unwrap()
                    .contains("Herman Melville")
            );
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_head_request() {
        let mock_server = MockServer::start().await;

        Mock::given(method("HEAD"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .insert_header("content-length", "100"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["method"], "HEAD");
            // HEAD requests should not have content
            assert!(value.get("content").is_none() || value["content"].is_null());
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_response_includes_size() {
        let mock_server = MockServer::start().await;
        let body = "<html><body>Test content</body></html>";

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            // Size should be present and > 0
            assert!(value["size"].as_u64().unwrap() > 0);
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_binary_returns_metadata() {
        let mock_server = MockServer::start().await;

        // Simulate a PNG image response
        Mock::given(method("GET"))
            .and(path("/image/png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]) // PNG magic bytes
                    .insert_header("content-type", "image/png")
                    .insert_header("content-length", "4"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/image/png", mock_server.uri())
            }))
            .await;

        // Binary content should return success with error message and metadata
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(
                value["content_type"]
                    .as_str()
                    .unwrap()
                    .contains("image/png")
            );
            assert!(
                value["error"].as_str().unwrap().contains("Binary content")
                    || value["error"].as_str().unwrap().contains("binary")
            );
            // Should have size metadata if available
            assert!(value.get("size").is_some() || value["size"].is_null());
        } else {
            panic!("Expected success response with metadata for binary content");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_truncated_field() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body>Short content</body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        // Normal response should have truncated: false
        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // truncated should be false or null for non-truncated content
            assert!(
                value["truncated"].is_null()
                    || value["truncated"] == false
                    || value.get("truncated").is_none()
            );
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_timeout_unreachable_host() {
        // Use TEST-NET-1 (192.0.2.0/24, RFC 5737) which is non-routable and will timeout.
        // Note: fetchkit v0.1.2 blocks RFC1918 private IPs, but TEST-NET ranges
        // are also blocked by DNS policy. Use a wiremock server with a delay instead.
        let mock_server = MockServer::start().await;

        // Mount a mock that takes 5 seconds to respond (exceeds 1s first-byte timeout)
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("slow response")
                    .set_delay(std::time::Duration::from_secs(5)),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/slow", mock_server.uri())
            }))
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("timed out") || msg.contains("connect") || msg.contains("failed"),
                    "Expected timeout or connection error, got: {}",
                    msg
                );
            }
            _ => {
                // Some environments may handle timeouts differently
            }
        }
    }

    #[tokio::test]
    async fn test_web_fetch_response_has_all_expected_fields() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body>Test</body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // Verify all expected fields are present
            assert!(value.get("url").is_some(), "Missing 'url' field");
            assert!(
                value.get("status_code").is_some(),
                "Missing 'status_code' field"
            );
            assert!(
                value.get("content_type").is_some(),
                "Missing 'content_type' field"
            );
            assert!(value.get("size").is_some(), "Missing 'size' field");
            // format, content may or may not be present depending on response type
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_head_response_structure() {
        let mock_server = MockServer::start().await;

        Mock::given(method("HEAD"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .insert_header("content-length", "100"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // HEAD response should have metadata but not content
            assert!(value.get("url").is_some());
            assert!(value.get("status_code").is_some());
            assert!(value.get("method").is_some());
            assert_eq!(value["method"], "HEAD");
            // Should NOT have content for HEAD
            assert!(value.get("content").is_none() || value["content"].is_null());
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_html_returns_markdown_by_default() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(
                        "<!DOCTYPE html><html><body><h1>Title</h1><p>Content</p></body></html>",
                    )
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        // No as_markdown needed - fetchkit returns markdown by default for HTML
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            // Content should be present
            let content = value["content"].as_str().unwrap();
            assert!(content.contains("Title") || content.contains("Content"));
            // Format should be "markdown" or "raw" depending on fetchkit's detection
            let format = value["format"].as_str().unwrap_or("raw");
            assert!(format == "markdown" || format == "raw");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_renders_inline_javascript_when_requested() {
        let mock_server = MockServer::start().await;
        let html = r#"<!doctype html>
            <html><body>
                <div id="app">Loading</div>
                <script>
                    document.body.innerHTML = '<main><h1>Rendered Inline</h1><p>Ready</p></main>';
                </script>
            </body></html>"#;

        Mock::given(method("GET"))
            .and(path("/spa"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html"))
            .mount(&mock_server)
            .await;

        let result = tool_for_wiremock()
            .execute(serde_json::json!({
                "url": format!("{}/spa", mock_server.uri()),
                "render": "rakers"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["rendered_by"], "rakers");
            let content = value["content"].as_str().unwrap();
            assert!(content.contains("Rendered Inline"));
            assert!(content.contains("Ready"));
            assert!(!content.contains("Loading"));
        } else {
            panic!("Expected rendered response, got: {result:?}");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_as_text_strips_html() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<!DOCTYPE html><html><body><b>Test</b> content</body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            // Content should be present
            let content = value["content"].as_str().unwrap();
            assert!(content.contains("Test") || content.contains("content"));
            // Format should be "text" or "raw" depending on fetchkit's detection
            let format = value["format"].as_str().unwrap_or("raw");
            assert!(format == "text" || format == "raw");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_raw_format_for_non_html() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("{\"key\": \"value\"}")
                    .insert_header("content-type", "application/json"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/json", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // JSON content should return "raw" format
            assert_eq!(value["format"], "raw");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_404_returns_success_with_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/status/404"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/status/404", mock_server.uri())
            }))
            .await;

        // 404 should still be a "success" from tool perspective - it got a response
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 404);
        } else {
            panic!("Expected successful response even for 404");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_500_returns_success_with_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/status/500"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/status/500", mock_server.uri())
            }))
            .await;

        // 500 should still be a "success" from tool perspective
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 500);
        } else {
            panic!("Expected successful response even for 500");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_dns_failure() {
        let tool = WebFetchTool::default();
        let result = tool
            .execute(serde_json::json!({
                "url": "https://this-domain-definitely-does-not-exist-12345.com/test"
            }))
            .await;

        // DNS failure returns a tool error. With fetchkit v0.1.2's resolve-then-check,
        // DNS resolution failures may surface as "blocked by policy" since the hostname
        // cannot be validated against the DNS policy.
        if let ToolExecutionResult::ToolError(msg) = result {
            let msg_lower = msg.to_lowercase();
            assert!(
                msg_lower.contains("failed")
                    || msg_lower.contains("error")
                    || msg_lower.contains("timed out")
                    || msg_lower.contains("connect")
                    || msg_lower.contains("blocked"),
                "Expected error message about failure, got: {}",
                msg
            );
        } else {
            // Some environments might timeout instead of DNS failure
        }
    }

    #[tokio::test]
    async fn test_web_fetch_rejects_ftp_url() {
        let tool = WebFetchTool::default();
        let result = tool
            .execute(serde_json::json!({
                "url": "ftp://example.com/file.txt"
            }))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid URL"));
        } else {
            panic!("Expected tool error for FTP URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_rejects_file_url() {
        let tool = WebFetchTool::default();
        let result = tool
            .execute(serde_json::json!({
                "url": "file:///etc/passwd"
            }))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid URL"));
        } else {
            panic!("Expected tool error for file:// URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_accepts_http_url() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/get"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("{\"url\": \"http://localhost/get\"}")
                    .insert_header("content-type", "application/json"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        // Note: mock_server.uri() returns http:// URL
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/get", mock_server.uri())
            }))
            .await;

        // HTTP (not HTTPS) should work
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
        } else {
            panic!("Expected successful response for HTTP URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_filters_excessive_newlines() {
        let mock_server = MockServer::start().await;

        // Response with many consecutive newlines
        Mock::given(method("GET"))
            .and(path("/newlines"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("line1\n\n\n\n\n\n\n\nline2")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/newlines", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            let content = value["content"].as_str().unwrap();
            // Should have at most 2 consecutive newlines
            assert!(
                !content.contains("\n\n\n"),
                "Content should not have more than 2 consecutive newlines"
            );
        } else {
            panic!("Expected successful response");
        }
    }

    // ========================================================================
    // SSRF security tests (TM-API-008 through TM-API-012)
    //
    // fetchkit v0.1.2 blocks private/internal IPs by default via
    // resolve-then-check with DNS pinning. These tests verify that
    // private/internal URLs are blocked by policy.
    //
    // Run with: cargo test -p everruns-core --lib -- web_fetch::tests::test_ssrf
    // ========================================================================

    // Helper: asserts that a private/internal URL IS blocked by fetchkit's
    // DNS policy (SSRF protection). The tool should return a ToolError
    // containing "blocked".
    async fn assert_blocked_by_policy(url: &str) {
        let tool = WebFetchTool::default();
        let result = tool.execute(serde_json::json!({"url": url})).await;
        assert!(
            matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("blocked")),
            "Expected URL {url} to be blocked by policy, got: {:?}",
            result
        );
    }

    /// THREAT[TM-API-009]: Cloud metadata endpoint blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_cloud_metadata_blocked() {
        assert_blocked_by_policy("http://169.254.169.254/latest/meta-data/").await;
    }

    /// THREAT[TM-API-008]: Localhost blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_localhost_blocked() {
        assert_blocked_by_policy("http://127.0.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 10.x.x.x blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_private_10_blocked() {
        assert_blocked_by_policy("http://10.0.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 172.16.x.x blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_private_172_blocked() {
        assert_blocked_by_policy("http://172.16.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 192.168.x.x blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_private_192_blocked() {
        assert_blocked_by_policy("http://192.168.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: IPv6 localhost blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_ipv6_localhost_blocked() {
        assert_blocked_by_policy("http://[::1]:1/").await;
    }

    /// THREAT[TM-API-008]: 0.0.0.0 blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_unspecified_blocked() {
        assert_blocked_by_policy("http://0.0.0.0:1/").await;
    }

    /// Verify file://, ftp://, gopher:// schemes are blocked (existing protection).
    #[tokio::test]
    async fn test_ssrf_non_http_schemes_blocked() {
        let tool = WebFetchTool::default();

        for (scheme, url) in [
            ("file://", "file:///etc/passwd"),
            ("ftp://", "ftp://internal-server/data"),
            ("gopher://", "gopher://internal-server/"),
        ] {
            let result = tool.execute(serde_json::json!({"url": url})).await;
            assert!(
                matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("Invalid URL")),
                "{scheme} should be rejected"
            );
        }
    }

    // ========================================================================
    // Integration tests using wiremock (no network access needed)
    // ========================================================================

    #[tokio::test]
    async fn test_fetch_html_page() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><head><title>Wasmtime Docs</title></head>
        <body><h1>Wasmtime</h1><p>A fast and secure runtime for WebAssembly.</p>
        <p>Wasmtime is a standalone runtime for WebAssembly that can be used
        as a CLI tool or embedded into other systems.</p></body></html>"#;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html; charset=utf-8"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.contains("Wasmtime") || content.contains("wasmtime"),
                "Content should mention Wasmtime"
            );
            assert!(
                value["size"].as_u64().unwrap() > 100,
                "Page should have substantial content"
            );
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_fetch_html_as_text() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><head><title>Wasmtime Docs</title></head>
        <body><h1>Wasmtime</h1><p>A fast and secure runtime.</p></body></html>"#;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.contains("Wasmtime") || content.contains("wasmtime"),
                "Text should contain Wasmtime reference"
            );
            let format = value["format"].as_str().unwrap_or("raw");
            assert!(
                format == "text" || format == "raw",
                "Format should be text or raw, got: {}",
                format
            );
        } else {
            panic!(
                "Expected successful response with text conversion, got: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_fetch_head_request() {
        let mock_server = MockServer::start().await;

        Mock::given(method("HEAD"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html; charset=utf-8")
                    .insert_header("content-length", "5000"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/", mock_server.uri()),
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["method"], "HEAD");
            assert!(
                value["content"].is_null()
                    || value["content"].as_str().is_none_or(|s| s.is_empty()),
                "HEAD request should not return content body"
            );
            assert!(value["content_type"].as_str().is_some());
        } else {
            panic!("Expected successful HEAD response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_fetch_subpage() {
        let mock_server = MockServer::start().await;
        // Build a page with >500 chars of content
        let body = format!(
            "<html><body><h1>Introduction</h1><p>{}</p></body></html>",
            "WebAssembly is a portable binary instruction format. ".repeat(20)
        );

        Mock::given(method("GET"))
            .and(path("/introduction.html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(&body)
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/introduction.html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.len() > 500,
                "Subpage should have substantial content, got {} bytes",
                content.len()
            );
        } else {
            panic!(
                "Expected successful response from subpage, got: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_fetch_repo_page() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><body>
        <h1>wasm3/wasm3</h1>
        <p>The fastest WebAssembly interpreter (and target for wasm3).</p>
        <div class="readme"><h2>README</h2><p>wasm3 is a high performance
        WebAssembly interpreter written in C.</p></div>
        </body></html>"#;

        Mock::given(method("GET"))
            .and(path("/wasm3/wasm3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html; charset=utf-8"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/wasm3/wasm3", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.to_lowercase().contains("wasm3"),
                "Content should mention wasm3"
            );
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_fetch_repo_page_as_text() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><body>
        <h1>wasm3/wasm3</h1>
        <p>The fastest WebAssembly interpreter written in C.</p>
        </body></html>"#;

        Mock::given(method("GET"))
            .and(path("/wasm3/wasm3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html; charset=utf-8"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/wasm3/wasm3", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.to_lowercase().contains("wasm3"),
                "Content should mention wasm3"
            );
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    // ========================================================================
    // File download tests (save_to_file via SessionFileSaver)
    // ========================================================================

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

    #[test]
    fn test_web_fetch_tool_schema_save_to_file_gated_by_config() {
        // Default (no file download): save_to_file NOT in schema
        let tool = WebFetchTool::new(false, None);
        let schema = tool.parameters_schema();
        assert!(
            !schema["properties"]["save_to_file"].is_object(),
            "Schema should NOT include save_to_file when disabled"
        );

        // With file download enabled: save_to_file in schema
        let tool = WebFetchTool::new(true, None);
        let schema = tool.parameters_schema();
        assert!(
            schema["properties"]["save_to_file"].is_object(),
            "Schema should include save_to_file when enabled"
        );
    }

    #[test]
    fn test_web_fetch_tool_requires_context() {
        let tool = WebFetchTool::default();
        assert!(tool.requires_context());
    }

    #[test]
    fn test_save_to_file_is_trimmed_and_blank_is_absent() {
        let blank = WebFetchTool::parse_request(&serde_json::json!({
            "url": "https://example.com",
            "save_to_file": "  \n\t "
        }))
        .unwrap();
        assert_eq!(blank.save_to_file, None);

        let path = WebFetchTool::parse_request(&serde_json::json!({
            "url": "https://example.com",
            "save_to_file": "  /downloads/file.txt  "
        }))
        .unwrap();
        assert_eq!(path.save_to_file.as_deref(), Some("/downloads/file.txt"));
    }

    #[test]
    fn test_fetchkit_request_fields_are_forwarded() {
        let request = WebFetchTool::parse_request(&serde_json::json!({
            "url": "https://example.com/docs",
            "method": "get",
            "content_focus": "agent",
            "crawl": true,
            "max_pages": 3,
            "if_none_match": "\"abc123\"",
            "if_modified_since": "Wed, 15 Jul 2026 12:00:00 GMT",
            "render": "rakers"
        }))
        .unwrap();

        assert_eq!(request.content_focus.as_deref(), Some("agent"));
        assert_eq!(request.crawl, Some(true));
        assert_eq!(request.max_pages, Some(3));
        assert_eq!(request.if_none_match.as_deref(), Some("\"abc123\""));
        assert_eq!(
            request.if_modified_since.as_deref(),
            Some("Wed, 15 Jul 2026 12:00:00 GMT")
        );
        assert_eq!(serde_json::to_value(request).unwrap()["render"], "rakers");
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
        context.egress_service = Some(Arc::new(CannedEgress));

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

    #[test]
    fn test_web_fetch_tools_with_config_enables_file_download() {
        let cap = WebFetchCapability::new(None);

        // Without config: no save_to_file in schema
        let tools = cap.tools_with_config(&serde_json::json!({}));
        assert_eq!(tools.len(), 1);
        let schema = tools[0].parameters_schema();
        assert!(!schema["properties"]["save_to_file"].is_object());

        // With enable_file_download: save_to_file in schema
        let tools = cap.tools_with_config(&serde_json::json!({"enable_file_download": true}));
        assert_eq!(tools.len(), 1);
        let schema = tools[0].parameters_schema();
        assert!(schema["properties"]["save_to_file"].is_object());
    }

    #[tokio::test]
    async fn test_web_fetch_system_prompt_adapts_to_config() {
        let cap = WebFetchCapability::new(None);
        let ctx = SystemPromptContext::without_file_store(SessionId::new());

        // Without file download: no save_to_file mention in prompt
        let prompt = cap
            .system_prompt_contribution_with_config(&ctx, &serde_json::json!({}))
            .await
            .unwrap();
        assert!(!prompt.contains("save_to_file"));

        // With file download: save_to_file documented in prompt
        let prompt = cap
            .system_prompt_contribution_with_config(
                &ctx,
                &serde_json::json!({"enable_file_download": true}),
            )
            .await
            .unwrap();
        assert!(prompt.contains("save_to_file"));
    }

    #[tokio::test]
    async fn test_save_to_file_text_content() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/data.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("{\"key\": \"value\"}")
                    .insert_header("content-type", "application/json"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let context = ToolContext::with_file_store(session_id, file_store.clone());

        let result = tool
            .execute_with_context(
                serde_json::json!({
                    "url": format!("{}/data.json", mock_server.uri()),
                    "save_to_file": "/downloads/data.json"
                }),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(value["saved_path"].as_str().is_some());
            assert!(value["bytes_written"].as_u64().unwrap() > 0);
            // Content should NOT be inline when saving to file
            assert!(
                value.get("content").is_none() || value["content"].is_null(),
                "Content should not be inline when saving to file"
            );

            // Verify file was written to the store
            let (content, encoding) = file_store
                .get_file(session_id, "/downloads/data.json")
                .await
                .expect("File should have been written");
            assert_eq!(encoding, "text");
            assert!(content.contains("value"));
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_session_file_saver_rejects_workspace_roots_and_directories() {
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        file_store.add_directory(session_id, "/downloads").await;
        let saver = SessionFileSaver {
            file_store,
            session_id,
        };

        for path in ["", "   ", "/", "/workspace", "/workspace/"] {
            let error = saver.validate_path(path).await.unwrap_err();
            assert!(
                matches!(error, FileSaveError::PathNotAllowed(_)),
                "root destination {path:?} should be a path error: {error}"
            );
        }

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
    async fn test_save_to_file_binary_content() {
        let mock_server = MockServer::start().await;

        // Serve a PNG image (binary content)
        let png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xFE];
        Mock::given(method("GET"))
            .and(path("/image.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(png_bytes.clone())
                    .insert_header("content-type", "image/png"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let context = ToolContext::with_file_store(session_id, file_store.clone());

        let result = tool
            .execute_with_context(
                serde_json::json!({
                    "url": format!("{}/image.png", mock_server.uri()),
                    "save_to_file": "/downloads/image.png"
                }),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(value["saved_path"].as_str().is_some());
            assert_eq!(
                value["bytes_written"].as_u64().unwrap(),
                png_bytes.len() as u64
            );

            // Verify file was written as base64 (binary content)
            let (content, encoding) = file_store
                .get_file(session_id, "/downloads/image.png")
                .await
                .expect("File should have been written");
            assert_eq!(encoding, "base64");

            // Decode and verify
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&content)
                .expect("Should be valid base64");
            assert_eq!(decoded, png_bytes);
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_save_to_file_no_file_store_returns_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/file.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("content"))
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        // Context without file_store
        let context = ToolContext::new(SessionId::new());

        let result = tool
            .execute_with_context(
                serde_json::json!({
                    "url": format!("{}/file.txt", mock_server.uri()),
                    "save_to_file": "/downloads/file.txt"
                }),
                &context,
            )
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(
                msg.contains("not available"),
                "Expected file system not available error, got: {}",
                msg
            );
        } else {
            panic!("Expected tool error, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_save_to_file_disabled_by_config_returns_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/file.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("content"))
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::new(false, None);
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let context = ToolContext::with_file_store(session_id, file_store.clone());

        let result = tool
            .execute_with_context(
                serde_json::json!({
                    "url": format!("{}/file.txt", mock_server.uri()),
                    "save_to_file": "/downloads/file.txt"
                }),
                &context,
            )
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(
                msg.contains("disabled"),
                "Expected file download disabled error, got: {}",
                msg
            );
        } else {
            panic!("Expected tool error, got: {:?}", result);
        }

        assert!(
            file_store
                .get_file(session_id, "/downloads/file.txt")
                .await
                .is_none(),
            "File should not be written when save_to_file is disabled",
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
            assert!(value["content"].as_str().is_some());
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
    struct CannedEgress;

    #[async_trait]
    impl crate::egress::EgressService for CannedEgress {
        async fn send(
            &self,
            _request: crate::egress::EgressRequest,
        ) -> crate::egress::EgressResult<crate::egress::EgressResponse> {
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
        context.egress_service = Some(Arc::new(CannedEgress));

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
        context.egress_service = Some(Arc::new(CannedEgress));
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
    }

    #[tokio::test]
    async fn test_egress_path_blocks_private_address_before_sending() {
        let tool = WebFetchTool::default();
        let mut context = ToolContext::new(SessionId::new());
        context.egress_service = Some(Arc::new(CannedEgress));

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
    }

    #[tokio::test]
    async fn test_egress_path_save_to_file_writes_session_file() {
        let tool = WebFetchTool::new(true, None);
        let file_store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let mut context = ToolContext::with_file_store(session_id, file_store.clone());
        context.egress_service = Some(Arc::new(CannedEgress));

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
            let (content, encoding) = file_store
                .get_file(session_id, "/downloads/file.txt")
                .await
                .expect("file should have been written via egress path");
            assert_eq!(encoding, "text");
            assert_eq!(content, "pong from egress");
        } else {
            panic!("Expected successful response, got: {result:?}");
        }
    }
}
