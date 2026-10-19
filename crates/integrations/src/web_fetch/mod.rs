#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
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
//! use everruns_contracts::runtime::Capability;
//! use everruns_integrations::web_fetch::WebFetchCapability;
//!
//! assert_eq!(WebFetchCapability::new(None).id(), "web_fetch");
//! ```

use self::tool_types::ToolHints;
use self::tools::{Tool, ToolExecutionResult};
use self::typed_id::SessionId;
use async_trait::async_trait;
use base64::Engine as _;
#[cfg(test)]
use everruns_contracts::error;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext,
};
use everruns_contracts::runtime::session_files::SessionFileSystem;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::*;
use everruns_contracts::{tool_types, typed_id};
use fetchkit::file_saver::{FileSaveError, FileSaver, SaveResult};
use fetchkit::{BotAuthConfig, FetchRequest};
use serde_json::Value;
use std::result::Result;
use std::sync::Arc;

mod credentials;
mod egress_transport;
mod fetch_error;
#[cfg(test)]
mod policy_tests;
mod request;

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

impl everruns_contracts::IntoCapability for WebFetch {
    fn into_capability(self) -> everruns_contracts::CapabilitySpec {
        everruns_contracts::CapabilityRef::new(WEB_FETCH_CAPABILITY_ID)
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
            "`web_fetch` fetches one URL (GET/HEAD) or sends one API request (POST etc. with headers and a body); it is not a search engine. For large or binary responses, pass `save_to_file` to write the body to the workspace instead of inlining it."
        } else {
            "`web_fetch` fetches one URL (GET/HEAD) or sends one API request (POST etc. with headers and a body); it is not a search engine."
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
                    "Отримує вміст за URL-адресою (GET/HEAD) або надсилає API-запит (POST тощо) і за потреби зберігає його у \
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
    /// Host-wide system egress policy (allowlist, deny list, mode),
    /// pre-checked on the initial URL for a clear, distinct system-policy
    /// error. On the egress path the boundary independently re-enforces it
    /// (final enforcement point, every hop); on the direct path this
    /// pre-flight is the only enforcement. `None` = no global enforcement.
    system_policy: Option<Arc<everruns_contracts::runtime::SystemEgressPolicy>>,
}

impl WebFetchTool {
    /// Create a new WebFetchTool with file download and optional bot-auth signing.
    pub fn new(enable_save_to_file: bool, bot_auth: Option<BotAuthConfig>) -> Self {
        // Decision: in-process JavaScript rendering (fetchkit's `render-rakers`)
        // is not enabled. It linked a JS engine and an HTML/CSS selector stack
        // into every binary for an opt-in fetch mode; JS-rendered pages are
        // served by the browserless integration instead.
        let mut builder = fetchkit::Tool::builder().enable_save_to_file(enable_save_to_file);
        if let Some(config) = bot_auth {
            builder = builder.bot_auth(config);
        }
        let fetchkit_tool = builder.build();
        let description = format!(
            "{}{}",
            fetchkit_tool.description(),
            request::DESCRIPTION_SUFFIX
        );
        Self {
            builder,
            fetchkit_tool,
            enable_save_to_file,
            description,
            system_policy: everruns_contracts::runtime::SystemEgressPolicy::from_env(),
        }
    }

    /// Reject a fetch (a GET read) the active system policy denies, with an
    /// explicit system-policy error. Returns `None` when no policy is active
    /// or the URL is permitted.
    fn system_policy_block(&self, url: &str) -> Option<ToolExecutionResult> {
        let policy = self.system_policy.as_ref()?;
        policy
            .check(url, everruns_contracts::runtime::EgressAccess::Read, None)
            .err()
            .map(|denial| ToolExecutionResult::tool_error(denial.message(url)))
    }

    /// Crawling can issue requests beyond the seed URL. The direct FetchKit
    /// transport cannot apply Everruns URL policy to those discovered pages.
    fn crawl_requires_egress(&self, request: &FetchRequest, context: Option<&ToolContext>) -> bool {
        request.crawl == Some(true)
            && (self.system_policy.is_some()
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
                return Err(ToolExecutionResult::tool_error(request::INVALID_METHOD));
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
}

#[async_trait]
impl Tool for WebFetchTool {
    fn narrate(
        &self,
        tool_call: &self::tool_types::ToolCall,
        phase: self::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: self::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(self::tool_narration::narrate_web_fetch(
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
        request::extend_schema(self.fetchkit_tool.input_schema())
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
        if request::is_raw(&arguments) {
            return request::execute(&arguments, None, self.system_policy.as_deref()).await;
        }
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
            Err(e) => fetch_error::map_error(e),
        }
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if request::is_raw(&arguments) {
            let policy = self.system_policy.as_deref();
            return request::execute(&arguments, Some(context), policy).await;
        }
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
                // The system policy is enforced again at the egress boundary,
                // but fetchkit resolves redirect targets before invoking the transport.
                // When a policy is active, keep redirects on the already
                // preflighted host so disallowed cross-host redirect labels cannot
                // leak via DNS before the boundary denies the request.
                let same_host_redirects_only = self.system_policy.is_some();
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
                Err(e) => fetch_error::map_error(e),
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
            Err(e) => fetch_error::map_error(e),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod credential_tests;
