//! Tool builder and toolkit-library contract for FetchKit.
//
// DECISION: keep the legacy typed `execute`/`llmtxt` surface as wrappers around the
// toolkit-library contract so existing fetchkit callers can migrate incrementally.

#[cfg(feature = "bot-auth")]
use crate::bot_auth::BotAuthConfig;
use crate::client::{fetch_with_options, FetchOptions};
use crate::dns::DnsPolicy;
use crate::error::{FetchError, ToolError};
use crate::fetchers::FetcherRegistry;
use crate::file_saver::FileSaver;
use crate::transport::HttpTransport;
use crate::types::{FetchRequest, FetchResponse};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::Service;

const DEFAULT_LOCALE: &str = "en-US";
const TOOL_NAME: &str = "web_fetch";

/// Status update during tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    /// Current phase (e.g., "validate", "connect", "fetch", "convert")
    pub phase: String,
    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Estimated completion percentage (0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent_complete: Option<f32>,
    /// Estimated time remaining in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_ms: Option<u64>,
}

impl ToolStatus {
    /// Create a new status with phase
    pub fn new(phase: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            message: None,
            percent_complete: None,
            eta_ms: None,
        }
    }

    /// Set message
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Set completion percentage
    pub fn with_percent(mut self, percent: f32) -> Self {
        self.percent_complete = Some(percent);
        self
    }

    /// Set ETA
    pub fn with_eta(mut self, eta_ms: u64) -> Self {
        self.eta_ms = Some(eta_ms);
        self
    }
}

/// Output image returned by the toolkit-library contract.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolImage {
    pub base64: String,
    pub media_type: String,
}

/// Consumer-only metadata returned by the toolkit-library contract.
#[derive(Debug, Clone)]
pub struct ToolOutputMetadata {
    pub duration: std::time::Duration,
    pub extra: Value,
}

impl Default for ToolOutputMetadata {
    fn default() -> Self {
        Self {
            duration: std::time::Duration::default(),
            extra: json!({}),
        }
    }
}

/// Structured tool output for the toolkit-library contract.
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub result: Value,
    pub images: Vec<ToolImage>,
    pub metadata: ToolOutputMetadata,
}

/// Builder for configuring the FetchKit tool
///
/// # Examples
///
/// ```
/// use fetchkit::ToolBuilder;
///
/// let tool = ToolBuilder::new()
///     .enable_markdown(true)
///     .enable_text(false)
///     .user_agent("MyBot/1.0")
///     .allow_prefix("https://docs.example.com")
///     .block_prefix("https://internal.example.com")
///     .build();
///
/// assert_eq!(tool.name(), "web_fetch");
/// ```
#[derive(Clone, Default)]
pub struct ToolBuilder {
    locale: String,
    /// Enable as_markdown option
    enable_markdown: bool,
    /// Enable as_text option
    enable_text: bool,
    /// Custom User-Agent
    user_agent: Option<String>,
    /// Allow list of URL prefixes
    allow_prefixes: Vec<String>,
    /// Block list of URL prefixes
    block_prefixes: Vec<String>,
    /// DNS resolution policy for SSRF prevention
    dns_policy: DnsPolicy,
    /// Maximum response body size in bytes
    max_body_size: Option<usize>,
    /// Enable save_to_file parameter (opt-in)
    enable_save_to_file: bool,
    /// Whether to honor proxy environment variables
    respect_proxy_env: bool,
    /// Restrict outbound requests to these ports. Empty means any port.
    allowed_ports: Vec<u16>,
    /// Block exact hosts and suffix rules. Leading '.' means suffix match.
    blocked_hosts: Vec<String>,
    /// Restrict redirects to the original host only.
    same_host_redirects_only: bool,
    /// Optional caller-owned URL policy evaluated before DNS resolution on every hop.
    url_policy: Option<Arc<dyn Fn(&url::Url) -> bool + Send + Sync>>,
    /// Web Bot Authentication config.
    #[cfg(feature = "bot-auth")]
    bot_auth: Option<BotAuthConfig>,
    /// Pluggable HTTP transport. None => default ReqwestTransport.
    transport: Option<Arc<dyn HttpTransport>>,
}

// Manual Debug: `transport` is a trait object that does not implement Debug
// (same pattern as FetchOptions).
impl std::fmt::Debug for ToolBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("ToolBuilder");
        d.field("locale", &self.locale)
            .field("enable_markdown", &self.enable_markdown)
            .field("enable_text", &self.enable_text)
            .field("user_agent", &self.user_agent)
            .field("allow_prefixes", &self.allow_prefixes)
            .field("block_prefixes", &self.block_prefixes)
            .field("dns_policy", &self.dns_policy)
            .field("max_body_size", &self.max_body_size)
            .field("enable_save_to_file", &self.enable_save_to_file)
            .field("respect_proxy_env", &self.respect_proxy_env)
            .field("allowed_ports", &self.allowed_ports)
            .field("blocked_hosts", &self.blocked_hosts)
            .field("same_host_redirects_only", &self.same_host_redirects_only)
            .field("url_policy", &self.url_policy.as_ref().map(|_| "<custom>"));
        #[cfg(feature = "bot-auth")]
        d.field("bot_auth", &self.bot_auth);
        d.field(
            "transport",
            &self
                .transport
                .as_ref()
                .map(|_| "<custom>")
                .unwrap_or("None"),
        );
        d.finish()
    }
}

impl ToolBuilder {
    /// Create a new tool builder with all options enabled
    pub fn new() -> Self {
        Self {
            locale: DEFAULT_LOCALE.to_string(),
            enable_markdown: true,
            enable_text: true,
            ..Default::default()
        }
    }

    /// Set the locale for user-facing metadata and tool errors.
    pub fn locale(mut self, locale: &str) -> Self {
        self.locale = normalize_locale(locale);
        self
    }

    /// Enable as_markdown option
    pub fn enable_markdown(mut self, enable: bool) -> Self {
        self.enable_markdown = enable;
        self
    }

    /// Enable as_text option
    pub fn enable_text(mut self, enable: bool) -> Self {
        self.enable_text = enable;
        self
    }

    /// Set custom User-Agent
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    /// Add URL prefix to allow list
    pub fn allow_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.allow_prefixes.push(prefix.into());
        self
    }

    /// Add URL prefix to block list
    pub fn block_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.block_prefixes.push(prefix.into());
        self
    }

    /// Set maximum response body size in bytes
    ///
    /// Limits the amount of data read from responses. Protects against
    /// memory exhaustion from large responses and compressed content bombs.
    /// Default: 10 MB if not set.
    pub fn max_body_size(mut self, size: usize) -> Self {
        self.max_body_size = Some(size);
        self
    }

    /// Enable file download (save_to_file parameter).
    /// Disabled by default — opt-in only.
    pub fn enable_save_to_file(mut self, enable: bool) -> Self {
        self.enable_save_to_file = enable;
        self
    }

    /// Allow outbound requests to a specific port.
    ///
    /// If no ports are configured, any URL port is allowed.
    pub fn allow_port(mut self, port: u16) -> Self {
        if !self.allowed_ports.contains(&port) {
            self.allowed_ports.push(port);
        }
        self
    }

    /// Block an exact hostname before DNS resolution.
    pub fn block_host(mut self, host: impl Into<String>) -> Self {
        self.blocked_hosts.push(host.into());
        self
    }

    /// Block a hostname suffix before DNS resolution.
    ///
    /// Suffixes should usually start with `.` such as `.cluster.local`.
    pub fn block_host_suffix(mut self, suffix: impl Into<String>) -> Self {
        let mut suffix = suffix.into();
        if !suffix.starts_with('.') {
            suffix.insert(0, '.');
        }
        self.blocked_hosts.push(suffix);
        self
    }

    /// Restrict redirects to the original host only.
    pub fn same_host_redirects_only(mut self, enable: bool) -> Self {
        self.same_host_redirects_only = enable;
        self
    }

    /// Restrict redirects to the original host only when the caller set a value.
    pub fn same_host_redirects_only_if_set(mut self, enable: Option<bool>) -> Self {
        if let Some(enable) = enable {
            self.same_host_redirects_only = enable;
        }
        self
    }

    /// Control private/reserved IP range blocking (SSRF prevention)
    ///
    /// Enabled by default. When enabled, FetchKit resolves hostnames to IP
    /// addresses before connecting and validates that the resolved IP is not
    /// in a private or reserved range. DNS pinning prevents rebinding attacks.
    ///
    /// Pass `false` only for local development or testing against loopback
    /// servers. In production, always leave this enabled.
    pub fn block_private_ips(mut self, block: bool) -> Self {
        self.dns_policy = if block {
            DnsPolicy::block_private_ips()
        } else {
            DnsPolicy::allow_all()
        };
        self
    }

    /// Control whether reqwest inherits proxy configuration from the process environment.
    ///
    /// Disabled by default so shared runtimes do not accidentally route requests
    /// through ambient HTTP(S)_PROXY configuration.
    pub fn respect_proxy_env(mut self, respect: bool) -> Self {
        self.respect_proxy_env = respect;
        self
    }

    /// Alias for [`respect_proxy_env`](Self::respect_proxy_env).
    pub fn use_env_proxy(mut self, enable: bool) -> Self {
        self.respect_proxy_env = enable;
        self
    }

    /// Set Web Bot Authentication config (draft-meunier-web-bot-auth-architecture).
    ///
    /// When configured, all outgoing requests are signed with Ed25519 per RFC 9421.
    /// Origins can verify bot identity via the `Signature` and `Signature-Input` headers.
    #[cfg(feature = "bot-auth")]
    pub fn bot_auth(mut self, config: BotAuthConfig) -> Self {
        self.bot_auth = Some(config);
        self
    }

    /// Set a caller-owned URL policy for every outbound hop before DNS resolution.
    pub fn url_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&url::Url) -> bool + Send + Sync + 'static,
    {
        self.url_policy = Some(Arc::new(policy));
        self
    }

    /// Set a custom HTTP transport for all tool execution paths.
    ///
    /// A host application can supply its own [`HttpTransport`] to route fetchkit's
    /// outbound HTTP through a dedicated egress boundary while keeping the Tool
    /// surface (description, schemas, llmtxt, `execute`, `execute_with_saver`).
    /// fetchkit still performs URL validation, DNS policy resolution, redirect
    /// following, bot-auth signing, and body/timeout caps; only the socket-level
    /// send is delegated. Default: the built-in `ReqwestTransport`.
    pub fn transport(mut self, transport: Arc<dyn HttpTransport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Apply a production-oriented hardening profile.
    ///
    /// This preset keeps private IP blocking enabled, ignores ambient proxy
    /// environment variables, restricts outbound traffic to ports 80 and 443,
    /// blocks common internal DNS suffixes, and only follows same-host redirects.
    pub fn hardened(mut self) -> Self {
        self = self
            .block_private_ips(true)
            .use_env_proxy(false)
            .allow_port(80)
            .allow_port(443)
            .block_host("localhost")
            .block_host_suffix(".local")
            .block_host_suffix(".internal")
            .block_host_suffix(".svc")
            .block_host_suffix(".cluster.local")
            .same_host_redirects_only(true);
        self
    }

    /// Build the full tool metadata + execution factory.
    pub fn build(&self) -> Tool {
        Tool {
            locale: self.locale.clone(),
            display_name: display_name(&self.locale).to_string(),
            description: description(&self.locale, self.enable_save_to_file),
            version: env!("CARGO_PKG_VERSION").to_string(),
            enable_markdown: self.enable_markdown,
            enable_text: self.enable_text,
            user_agent: self.user_agent.clone(),
            allow_prefixes: self.allow_prefixes.clone(),
            block_prefixes: self.block_prefixes.clone(),
            dns_policy: self.dns_policy.clone(),
            max_body_size: self.max_body_size,
            enable_save_to_file: self.enable_save_to_file,
            respect_proxy_env: self.respect_proxy_env,
            allowed_ports: self.allowed_ports.clone(),
            blocked_hosts: self.blocked_hosts.clone(),
            same_host_redirects_only: self.same_host_redirects_only,
            url_policy: self.url_policy.clone(),
            #[cfg(feature = "bot-auth")]
            bot_auth: self.bot_auth.clone(),
            transport: self.transport.clone(),
        }
    }

    /// Build a `tower::Service` that accepts JSON args and returns JSON result.
    pub fn build_service(&self) -> ToolService {
        ToolService { tool: self.build() }
    }

    /// Alias for `build_service()` for generic executor-oriented consumers.
    pub fn build_executor(&self) -> ToolService {
        self.build_service()
    }

    /// Build an OpenAI-compatible function tool definition.
    pub fn build_tool_definition(&self) -> Value {
        let tool = self.build();
        json!({
            "type": "function",
            "function": {
                "name": tool.name(),
                "description": tool.description(),
                "parameters": tool.input_schema()
            }
        })
    }

    /// Build the JSON Schema for the tool's input parameters.
    pub fn build_input_schema(&self) -> Value {
        build_input_schema(
            self.enable_markdown,
            self.enable_text,
            self.enable_save_to_file,
        )
    }

    /// Build the JSON Schema for the tool's output.
    pub fn build_output_schema(&self) -> Value {
        build_output_schema()
    }
}

/// Configured FetchKit tool
///
/// Created via [`ToolBuilder`]. Provides methods for executing fetch requests,
/// retrieving schemas, and accessing tool metadata.
#[derive(Clone)]
pub struct Tool {
    locale: String,
    display_name: String,
    description: String,
    version: String,
    enable_markdown: bool,
    enable_text: bool,
    user_agent: Option<String>,
    allow_prefixes: Vec<String>,
    block_prefixes: Vec<String>,
    dns_policy: DnsPolicy,
    max_body_size: Option<usize>,
    enable_save_to_file: bool,
    respect_proxy_env: bool,
    allowed_ports: Vec<u16>,
    blocked_hosts: Vec<String>,
    same_host_redirects_only: bool,
    url_policy: Option<Arc<dyn Fn(&url::Url) -> bool + Send + Sync>>,
    #[cfg(feature = "bot-auth")]
    bot_auth: Option<BotAuthConfig>,
    transport: Option<Arc<dyn HttpTransport>>,
}

// Manual Debug: `transport` is a trait object that does not implement Debug
// (same pattern as FetchOptions).
impl std::fmt::Debug for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Tool");
        d.field("locale", &self.locale)
            .field("display_name", &self.display_name)
            .field("description", &self.description)
            .field("version", &self.version)
            .field("enable_markdown", &self.enable_markdown)
            .field("enable_text", &self.enable_text)
            .field("user_agent", &self.user_agent)
            .field("allow_prefixes", &self.allow_prefixes)
            .field("block_prefixes", &self.block_prefixes)
            .field("dns_policy", &self.dns_policy)
            .field("max_body_size", &self.max_body_size)
            .field("enable_save_to_file", &self.enable_save_to_file)
            .field("respect_proxy_env", &self.respect_proxy_env)
            .field("allowed_ports", &self.allowed_ports)
            .field("blocked_hosts", &self.blocked_hosts)
            .field("same_host_redirects_only", &self.same_host_redirects_only)
            .field("url_policy", &self.url_policy.as_ref().map(|_| "<custom>"));
        #[cfg(feature = "bot-auth")]
        d.field("bot_auth", &self.bot_auth);
        d.field(
            "transport",
            &self
                .transport
                .as_ref()
                .map(|_| "<custom>")
                .unwrap_or("None"),
        );
        d.finish()
    }
}

impl Default for Tool {
    fn default() -> Self {
        ToolBuilder::new().build()
    }
}

impl Tool {
    /// Create a new tool builder
    pub fn builder() -> ToolBuilder {
        ToolBuilder::new()
    }

    /// Tool name for LLM invocation.
    pub fn name(&self) -> &str {
        TOOL_NAME
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Toolkit version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Tool description, reflecting enabled features.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Tool locale.
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// Get system prompt contribution.
    pub fn system_prompt(&self) -> String {
        system_prompt(
            &self.locale,
            self.enable_save_to_file,
            self.dns_policy.block_private,
        )
    }

    /// Get comprehensive Markdown help.
    pub fn help(&self) -> String {
        build_help(self)
    }

    /// Backward-compatible alias for `help()`.
    pub fn llmtxt(&self) -> String {
        self.help()
    }

    /// Get input schema as JSON.
    pub fn input_schema(&self) -> Value {
        build_input_schema(
            self.enable_markdown,
            self.enable_text,
            self.enable_save_to_file,
        )
    }

    /// Get output schema as JSON.
    pub fn output_schema(&self) -> Value {
        build_output_schema()
    }

    /// Create a single-use tool execution from JSON arguments.
    pub fn execution(&self, args: Value) -> Result<ToolExecution, ToolError> {
        validate_args(self, &args)?;
        let request: FetchRequest = serde_json::from_value(args)
            .map_err(|err| invalid_arguments_error(self.locale(), &err.to_string()))?;
        validate_request(self, &request)?;

        Ok(ToolExecution {
            tool: self.clone(),
            request,
        })
    }

    /// Execute the tool with the given typed request.
    pub async fn execute(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        fetch_with_options(req, self.build_options()).await
    }

    /// Execute the tool with status updates.
    pub async fn execute_with_status<F>(
        &self,
        req: FetchRequest,
        mut status_callback: F,
    ) -> Result<FetchResponse, FetchError>
    where
        F: FnMut(ToolStatus),
    {
        status_callback(ToolStatus::new("validate").with_percent(0.0));

        if req.url.is_empty() {
            return Err(FetchError::MissingUrl);
        }

        if !req.url.starts_with("http://") && !req.url.starts_with("https://") {
            return Err(FetchError::InvalidUrlScheme);
        }

        status_callback(ToolStatus::new("connect").with_percent(10.0));
        status_callback(ToolStatus::new("fetch").with_percent(20.0));

        let result = fetch_with_options(req, self.build_options()).await;

        status_callback(ToolStatus::new("complete").with_percent(100.0));

        result
    }

    /// Execute fetch with optional file saving.
    ///
    /// When `req.save_to_file` is set, validates the path via the saver,
    /// fetches content (including binary), and saves through the saver.
    /// Returns metadata without inline content.
    ///
    /// When `req.save_to_file` is `None`, behaves identically to [`execute`](Self::execute).
    pub async fn execute_with_saver(
        &self,
        req: FetchRequest,
        saver: Option<&dyn FileSaver>,
    ) -> Result<FetchResponse, FetchError> {
        if req.save_to_file.is_some() {
            if !self.enable_save_to_file {
                return Err(FetchError::SaverNotAvailable);
            }

            let saver = saver.ok_or(FetchError::SaverNotAvailable)?;

            let options = self.build_options();
            let registry = FetcherRegistry::with_defaults();
            registry.fetch_to_file(req, options, saver).await
        } else {
            self.execute(req).await
        }
    }

    fn build_options(&self) -> FetchOptions {
        FetchOptions {
            user_agent: self.user_agent.clone(),
            allow_prefixes: self.allow_prefixes.clone(),
            block_prefixes: self.block_prefixes.clone(),
            enable_markdown: self.enable_markdown,
            enable_text: self.enable_text,
            dns_policy: self.dns_policy.clone(),
            max_body_size: self.max_body_size,
            enable_save_to_file: self.enable_save_to_file,
            respect_proxy_env: self.respect_proxy_env,
            allowed_ports: self.allowed_ports.clone(),
            blocked_hosts: self.blocked_hosts.clone(),
            same_host_redirects_only: self.same_host_redirects_only,
            url_policy: self.url_policy.clone(),
            #[cfg(feature = "bot-auth")]
            bot_auth: self.bot_auth.clone(),
            // None => default ReqwestTransport; hosts inject a custom transport via
            // ToolBuilder::transport and every Tool execution path honors it.
            transport: self.transport.clone(),
        }
    }
}

/// Single-use runtime execution for one tool call.
#[derive(Debug, Clone)]
pub struct ToolExecution {
    tool: Tool,
    request: FetchRequest,
}

impl ToolExecution {
    /// Run to completion without adapters.
    pub async fn execute(self) -> Result<ToolOutput, ToolError> {
        self.execute_inner(None).await
    }

    /// Run to completion with an injected file saver adapter.
    pub async fn execute_with<A>(self, saver: &A) -> Result<ToolOutput, ToolError>
    where
        A: FileSaver,
    {
        self.execute_inner(Some(saver)).await
    }

    async fn execute_inner(self, saver: Option<&dyn FileSaver>) -> Result<ToolOutput, ToolError> {
        let ToolExecution { tool, request } = self;
        let started_at = Instant::now();
        let response = tool
            .execute_with_saver(request, saver)
            .await
            .map_err(|err| map_fetch_error(&tool.locale, err))?;

        build_tool_output(response, started_at)
    }
}

/// Generic JSON args → JSON result service.
#[derive(Debug, Clone)]
pub struct ToolService {
    tool: Tool,
}

impl Service<Value> for ToolService {
    type Response = Value;
    type Error = ToolError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Value) -> Self::Future {
        let tool = self.tool.clone();
        Box::pin(async move {
            let output = tool.execution(req)?.execute().await?;
            Ok(output.result)
        })
    }
}

fn build_tool_output(
    response: FetchResponse,
    started_at: Instant,
) -> Result<ToolOutput, ToolError> {
    let result = serde_json::to_value(&response)
        .map_err(|err| ToolError::Internal(format!("failed to serialize tool output: {err}")))?;

    Ok(ToolOutput {
        result,
        images: Vec::new(),
        metadata: ToolOutputMetadata {
            duration: started_at.elapsed(),
            extra: json!({
                "http_status": response.status_code,
                "content_type": response.content_type,
                "content_length": response.size,
                "format": response.format,
                "truncated": response.truncated.unwrap_or(false),
                "saved_path": response.saved_path,
                "bytes_written": response.bytes_written,
            }),
        },
    })
}

fn validate_args(tool: &Tool, args: &Value) -> Result<(), ToolError> {
    let object = args
        .as_object()
        .ok_or_else(|| invalid_arguments_error(tool.locale(), "arguments must be a JSON object"))?;

    for key in object.keys() {
        let allowed = match key.as_str() {
            "url" | "method" => true,
            "as_markdown" => tool.enable_markdown,
            "as_text" => tool.enable_text,
            "save_to_file" => tool.enable_save_to_file,
            "if_none_match" | "if_modified_since" => true,
            _ => false,
        };

        if !allowed {
            return Err(unknown_parameter_error(tool.locale(), key));
        }
    }

    Ok(())
}

fn validate_request(tool: &Tool, request: &FetchRequest) -> Result<(), ToolError> {
    if request.url.is_empty() {
        return Err(map_fetch_error(tool.locale(), FetchError::MissingUrl));
    }

    if !request.url.starts_with("http://") && !request.url.starts_with("https://") {
        return Err(map_fetch_error(tool.locale(), FetchError::InvalidUrlScheme));
    }

    Ok(())
}

fn build_input_schema(
    enable_markdown: bool,
    enable_text: bool,
    enable_save_to_file: bool,
) -> Value {
    let mut properties = Map::new();
    properties.insert(
        "url".to_string(),
        json!({"type": "string", "format": "uri"}),
    );
    properties.insert(
        "method".to_string(),
        json!({"type": "string", "enum": ["GET", "HEAD"], "default": "GET"}),
    );

    if enable_markdown {
        properties.insert(
            "as_markdown".to_string(),
            json!({"type": "boolean", "default": false}),
        );
    }

    if enable_text {
        properties.insert(
            "as_text".to_string(),
            json!({"type": "boolean", "default": false}),
        );
    }

    if enable_save_to_file {
        properties.insert(
            "save_to_file".to_string(),
            json!({
                "type": "string",
                "description": "Adapter-defined destination path"
            }),
        );
    }

    properties.insert(
        "if_none_match".to_string(),
        json!({
            "type": "string",
            "description": "ETag value for conditional requests (If-None-Match header)"
        }),
    );
    properties.insert(
        "if_modified_since".to_string(),
        json!({
            "type": "string",
            "description": "Last-Modified value for conditional requests (If-Modified-Since header)"
        }),
    );

    json!({
        "type": "object",
        "properties": properties,
        "required": ["url"],
        "additionalProperties": false,
    })
}

fn build_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "url": {"type": "string"},
            "status_code": {"type": "integer", "minimum": 100, "maximum": 599},
            "content_type": {"type": "string"},
            "size": {"type": "integer", "minimum": 0},
            "last_modified": {"type": "string"},
            "etag": {"type": "string"},
            "filename": {"type": "string"},
            "format": {"type": "string", "enum": ["markdown", "text", "raw", "github_repo"]},
            "content": {"type": "string"},
            "truncated": {"type": "boolean"},
            "method": {"type": "string", "enum": ["HEAD"]},
            "error": {"type": "string"},
            "saved_path": {"type": "string"},
            "bytes_written": {"type": "integer", "minimum": 0},
            "word_count": {"type": "integer", "minimum": 0},
            "redirect_chain": {"type": "array", "items": {"type": "string"}},
            "is_paywall": {"type": "boolean"}
        },
        "required": ["url", "status_code"],
        "additionalProperties": false
    })
}

fn normalize_locale(locale: &str) -> String {
    let locale = locale.trim();
    if locale.is_empty() {
        DEFAULT_LOCALE.to_string()
    } else {
        locale.to_string()
    }
}

fn is_ukrainian(locale: &str) -> bool {
    locale.to_ascii_lowercase().starts_with("uk")
}

fn display_name(locale: &str) -> &'static str {
    if is_ukrainian(locale) {
        "Веб-завантаження"
    } else {
        "Web Fetch"
    }
}

fn description(locale: &str, enable_save_to_file: bool) -> String {
    if is_ukrainian(locale) {
        if enable_save_to_file {
            "Завантажити URL як текст або markdown; повернути метадані для бінарного вмісту або зберегти байти через save_to_file.".to_string()
        } else {
            "Завантажити URL як текст або markdown; повернути метадані для бінарного вмісту."
                .to_string()
        }
    } else if enable_save_to_file {
        "Fetch URL content as text or markdown; return metadata for binary responses or save bytes with save_to_file.".to_string()
    } else {
        "Fetch URL content as text or markdown; return metadata for binary responses.".to_string()
    }
}

fn system_prompt(locale: &str, enable_save_to_file: bool, block_private_ips: bool) -> String {
    if is_ukrainian(locale) {
        let binary_rule = if enable_save_to_file {
            "Бінарні відповіді повертають метадані; використовуйте save_to_file, щоб зберегти байти."
        } else {
            "Бінарні відповіді повертають лише метадані."
        };
        let network_rule = if block_private_ips {
            "Приватні IP-адреси заблоковані."
        } else {
            "Блокування приватних IP-адрес вимкнене."
        };
        format!(
            "{}: повертає truncated=true для часткових відповідей після таймауту. {} {}",
            TOOL_NAME, binary_rule, network_rule
        )
    } else {
        let binary_rule = if enable_save_to_file {
            "Binary responses return metadata; use save_to_file to persist bytes."
        } else {
            "Binary responses return metadata only."
        };
        let network_rule = if block_private_ips {
            "Private IPs are blocked."
        } else {
            "Private IP blocking is disabled."
        };
        format!(
            "{}: returns truncated=true for partial responses after timeout. {} {}",
            TOOL_NAME, binary_rule, network_rule
        )
    }
}

fn build_help(tool: &Tool) -> String {
    let (parameters_heading, examples_heading, adapters_heading, errors_heading, locale_label) =
        if is_ukrainian(tool.locale()) {
            ("Параметри", "Приклади", "Адаптери", "Помилки", "Локаль")
        } else {
            ("Parameters", "Examples", "Adapters", "Errors", "Locale")
        };

    let mut rows = vec![
        table_row(
            "url",
            "string",
            "yes",
            "—",
            parameter_description(tool.locale(), "url"),
        ),
        table_row(
            "method",
            "string",
            "no",
            "\"GET\"",
            parameter_description(tool.locale(), "method"),
        ),
    ];

    if tool.enable_markdown {
        rows.push(table_row(
            "as_markdown",
            "boolean",
            "no",
            "false",
            parameter_description(tool.locale(), "as_markdown"),
        ));
    }

    if tool.enable_text {
        rows.push(table_row(
            "as_text",
            "boolean",
            "no",
            "false",
            parameter_description(tool.locale(), "as_text"),
        ));
    }

    if tool.enable_save_to_file {
        rows.push(table_row(
            "save_to_file",
            "string",
            "no",
            "—",
            parameter_description(tool.locale(), "save_to_file"),
        ));
    }

    let adapters = if tool.enable_save_to_file {
        if is_ukrainian(tool.locale()) {
            "- `FileSaver` (необов’язковий): потрібен, коли задано `save_to_file`.\n"
        } else {
            "- `FileSaver` (optional): required when `save_to_file` is set.\n"
        }
    } else if is_ukrainian(tool.locale()) {
        "- Збереження файлів вимкнене в цій конфігурації.\n"
    } else {
        "- File saving is disabled in this tool build.\n"
    };

    let errors = if is_ukrainian(tool.locale()) {
        if tool.enable_save_to_file {
            "- `MissingUrl` — параметр `url` обов’язковий\n\
             - `InvalidUrlScheme` — схема URL має бути `http` або `https`\n\
             - `BlockedUrl` — URL заблокований політикою SSRF або allow/block правилами\n\
             - `FirstByteTimeout` — сервер не відповів протягом 1 секунди\n\
             - `SaverNotAvailable` — `save_to_file` потребує адаптер `FileSaver`\n"
        } else {
            "- `MissingUrl` — параметр `url` обов’язковий\n\
             - `InvalidUrlScheme` — схема URL має бути `http` або `https`\n\
             - `BlockedUrl` — URL заблокований політикою SSRF або allow/block правилами\n\
             - `FirstByteTimeout` — сервер не відповів протягом 1 секунди\n"
        }
    } else if tool.enable_save_to_file {
        "- `MissingUrl` — `url` is required\n\
         - `InvalidUrlScheme` — URL scheme must be `http` or `https`\n\
         - `BlockedUrl` — URL blocked by SSRF policy or allow/block rules\n\
         - `FirstByteTimeout` — server did not respond within 1 second\n\
         - `SaverNotAvailable` — `save_to_file` requires a `FileSaver` adapter\n"
    } else {
        "- `MissingUrl` — `url` is required\n\
         - `InvalidUrlScheme` — URL scheme must be `http` or `https`\n\
         - `BlockedUrl` — URL blocked by SSRF policy or allow/block rules\n\
         - `FirstByteTimeout` — server did not respond within 1 second\n"
    };

    let mut help = String::new();
    help.push_str(&format!("# {}\n\n", tool.display_name()));
    help.push_str(tool.description());
    help.push_str("\n\n");
    help.push_str(&format!("**Version:** {}\n", tool.version()));
    help.push_str(&format!("**Name:** `{}`\n", tool.name()));
    help.push_str(&format!("**{}:** `{}`\n\n", locale_label, tool.locale()));
    help.push_str(&format!("## {}\n\n", parameters_heading));
    help.push_str("| Name | Type | Required | Default | Description |\n");
    help.push_str("|------|------|----------|---------|-------------|\n");
    for row in rows {
        help.push_str(&row);
    }
    help.push('\n');
    help.push_str(&format!("## {}\n\n", examples_heading));
    help.push_str("```json\n");
    help.push_str("{\"url\": \"https://example.com\", \"as_markdown\": true}\n");
    help.push_str("```\n\n");
    help.push_str("```json\n");
    help.push_str("{\"url\": \"https://example.com/file.pdf\", \"method\": \"HEAD\"}\n");
    help.push_str("```\n");
    if tool.enable_save_to_file {
        help.push_str("\n```json\n");
        help.push_str(
            "{\"url\": \"https://example.com/image.png\", \"save_to_file\": \"/tmp/image.png\"}\n",
        );
        help.push_str("```\n");
    }
    help.push('\n');
    help.push_str(&format!("## {}\n\n", adapters_heading));
    help.push_str(adapters);
    help.push('\n');
    help.push_str(&format!("## {}\n\n", errors_heading));
    help.push_str(errors);
    help.push('\n');
    help.push_str("## System Prompt\n\n");
    help.push_str(&tool.system_prompt());
    help.push('\n');
    help
}

fn parameter_description(locale: &str, field: &str) -> &'static str {
    match (is_ukrainian(locale), field) {
        (true, "url") => "HTTP або HTTPS URL",
        (true, "method") => "`GET` або `HEAD`",
        (true, "as_markdown") => "Перетворити HTML у markdown",
        (true, "as_text") => "Перетворити HTML у plain text",
        (true, "save_to_file") => "Шлях призначення, визначений адаптером",
        (false, "url") => "HTTP or HTTPS URL",
        (false, "method") => "`GET` or `HEAD`",
        (false, "as_markdown") => "Convert HTML to markdown",
        (false, "as_text") => "Convert HTML to plain text",
        (false, "save_to_file") => "Adapter-defined destination path",
        _ => "",
    }
}

fn table_row(name: &str, ty: &str, required: &str, default: &str, description: &str) -> String {
    format!("| `{name}` | {ty} | {required} | {default} | {description} |\n")
}

fn map_fetch_error(locale: &str, err: FetchError) -> ToolError {
    match err {
        FetchError::ClientBuildError(_) => internal_error("failed to create HTTP client"),
        FetchError::MissingUrl => user_error(locale, user_text(locale, "missing_url")),
        FetchError::InvalidUrlScheme => user_error(locale, user_text(locale, "invalid_scheme")),
        FetchError::InvalidMethod => user_error(locale, user_text(locale, "invalid_method")),
        FetchError::BlockedUrl => user_error(locale, user_text(locale, "blocked_url")),
        FetchError::FirstByteTimeout => user_error(locale, user_text(locale, "timeout")),
        FetchError::ConnectError(_) => user_error(locale, user_text(locale, "connect_error")),
        FetchError::RequestError(_) => user_error(locale, user_text(locale, "request_error")),
        FetchError::FetcherError(_) => user_error(locale, user_text(locale, "fetcher_error")),
        FetchError::SaveError(_) => user_error(locale, user_text(locale, "save_error")),
        FetchError::SaverNotAvailable => user_error(locale, user_text(locale, "saver_missing")),
    }
}

fn invalid_arguments_error(locale: &str, detail: &str) -> ToolError {
    if is_ukrainian(locale) {
        user_error(locale, format!("Неприпустимі аргументи: {detail}"))
    } else {
        user_error(locale, format!("Invalid arguments: {detail}"))
    }
}

fn unknown_parameter_error(locale: &str, key: &str) -> ToolError {
    if is_ukrainian(locale) {
        user_error(locale, format!("Невідомий параметр: {key}"))
    } else {
        user_error(locale, format!("Unknown parameter: {key}"))
    }
}

fn user_text(locale: &str, key: &str) -> &'static str {
    match (is_ukrainian(locale), key) {
        (true, "missing_url") => "Параметр url обов’язковий.",
        (true, "invalid_scheme") => "Схема URL має бути http або https.",
        (true, "invalid_method") => "Метод має бути GET або HEAD.",
        (true, "blocked_url") => "URL заблокований політикою безпеки.",
        (true, "timeout") => {
            "Сервер не відповів протягом 1 секунди. Спробуйте ще раз або інший URL."
        }
        (true, "connect_error") => {
            "Не вдалося з’єднатися із сервером. Спробуйте ще раз або інший URL."
        }
        (true, "request_error") => "Запит не вдався. Спробуйте ще раз.",
        (true, "fetcher_error") => "Не вдалося обробити відповідь цього URL.",
        (true, "save_error") => "Не вдалося зберегти файл. Перевірте шлях призначення.",
        (true, "saver_missing") => "save_to_file потребує адаптер FileSaver.",
        (false, "missing_url") => "url is required.",
        (false, "invalid_scheme") => "URL scheme must be http or https.",
        (false, "invalid_method") => "Method must be GET or HEAD.",
        (false, "blocked_url") => "URL is blocked by security policy.",
        (false, "timeout") => {
            "Server did not respond within 1 second. Retry or try a different URL."
        }
        (false, "connect_error") => "Could not connect to server. Retry or try a different URL.",
        (false, "request_error") => "Request failed. Retry the tool call.",
        (false, "fetcher_error") => "Could not process the response for this URL.",
        (false, "save_error") => "Could not save the file. Check the destination path.",
        (false, "saver_missing") => "save_to_file requires the FileSaver adapter.",
        _ => "Tool execution failed.",
    }
}

fn user_error(locale: &str, message: impl Into<String>) -> ToolError {
    let _ = locale;
    ToolError::UserFacing(message.into())
}

fn internal_error(message: impl Into<String>) -> ToolError {
    ToolError::Internal(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tower::Service;

    #[test]
    fn test_tool_builder() {
        let tool = Tool::builder()
            .locale("uk-UA")
            .enable_markdown(false)
            .enable_text(true)
            .user_agent("TestAgent/1.0")
            .allow_prefix("https://allowed.com")
            .block_prefix("https://blocked.com")
            .max_body_size(1024)
            .respect_proxy_env(true)
            .build();

        assert_eq!(tool.locale(), "uk-UA");
        assert_eq!(tool.name(), "web_fetch");
        assert_eq!(tool.display_name(), "Веб-завантаження");
        assert!(!tool.enable_markdown);
        assert!(tool.enable_text);
        assert_eq!(tool.user_agent, Some("TestAgent/1.0".to_string()));
        assert_eq!(tool.allow_prefixes, vec!["https://allowed.com"]);
        assert_eq!(tool.block_prefixes, vec!["https://blocked.com"]);
        assert!(tool.dns_policy.block_private);
        assert_eq!(tool.max_body_size, Some(1024));
        assert!(!tool.enable_save_to_file);
        assert!(tool.respect_proxy_env);
        assert!(tool.allowed_ports.is_empty());
        assert!(tool.blocked_hosts.is_empty());
        assert!(!tool.same_host_redirects_only);
    }

    #[test]
    fn test_tool_builder_opt_out_private_ip_blocking() {
        let tool = Tool::builder().block_private_ips(false).build();
        assert!(!tool.dns_policy.block_private);
    }

    #[test]
    fn test_tool_builder_security_defaults() {
        let tool = Tool::builder().build();
        assert!(tool.max_body_size.is_none());
        assert!(!tool.enable_save_to_file);
        assert!(!tool.respect_proxy_env);
        assert!(tool.allowed_ports.is_empty());
        assert!(tool.blocked_hosts.is_empty());
        assert!(!tool.same_host_redirects_only);
    }

    #[test]
    fn test_tool_builder_hardened_profile() {
        let tool = Tool::builder().hardened().build();

        assert!(tool.dns_policy.block_private);
        assert!(!tool.respect_proxy_env);
        assert_eq!(tool.allowed_ports, vec![80, 443]);
        assert!(tool.blocked_hosts.contains(&"localhost".to_string()));
        assert!(tool.blocked_hosts.contains(&".cluster.local".to_string()));
        assert!(tool.same_host_redirects_only);
    }

    #[test]
    fn test_tool_builder_preserves_hardened_redirect_policy_when_override_is_unset() {
        let tool = Tool::builder()
            .hardened()
            .same_host_redirects_only_if_set(None)
            .build();

        assert!(tool.same_host_redirects_only);
    }

    #[test]
    fn test_tool_builder_allows_explicit_redirect_policy_override() {
        let tool = Tool::builder()
            .hardened()
            .same_host_redirects_only_if_set(Some(false))
            .build();

        assert!(!tool.same_host_redirects_only);
    }

    #[test]
    fn test_tool_metadata() {
        let tool = Tool::default();
        assert_eq!(tool.name(), "web_fetch");
        assert_eq!(tool.display_name(), "Web Fetch");
        assert!(!tool.description().is_empty());
        assert!(tool.system_prompt().starts_with("web_fetch:"));
        assert!(tool.help().contains("## Parameters"));
        assert_eq!(tool.locale(), "en-US");
    }

    #[test]
    fn test_tool_llmtxt_matches_help() {
        let tool = Tool::builder().enable_save_to_file(true).build();
        assert_eq!(tool.llmtxt(), tool.help());
        assert!(tool.llmtxt().contains("save_to_file"));
    }

    #[test]
    fn test_tool_schemas() {
        let tool = Tool::default();
        let input_schema = tool.input_schema();
        let output_schema = tool.output_schema();

        assert_eq!(input_schema["type"], "object");
        assert_eq!(input_schema["properties"]["url"]["format"], "uri");
        assert_eq!(input_schema["properties"]["method"]["default"], "GET");
        assert!(input_schema["properties"]["if_none_match"].is_object());
        assert!(input_schema["properties"]["if_modified_since"].is_object());
        assert!(output_schema["properties"]["url"].is_object());
        assert!(output_schema["properties"]["status_code"].is_object());
        assert!(output_schema["properties"]["word_count"].is_object());
        assert!(output_schema["properties"]["redirect_chain"].is_object());
        assert!(output_schema["properties"]["is_paywall"].is_object());
        assert!(output_schema["properties"]["etag"].is_object());
    }

    #[test]
    fn test_tool_schema_feature_gating() {
        let tool = Tool::builder()
            .enable_markdown(false)
            .enable_text(false)
            .build();

        let schema = tool.input_schema();
        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .unwrap();

        assert!(!props.contains_key("as_markdown"));
        assert!(!props.contains_key("as_text"));
    }

    #[test]
    fn test_tool_definition_uses_contract_metadata() {
        let definition = Tool::builder()
            .enable_save_to_file(true)
            .build_tool_definition();
        assert_eq!(definition["type"], "function");
        assert_eq!(definition["function"]["name"], "web_fetch");
        assert_eq!(definition["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn test_execution_rejects_unknown_parameter() {
        let err = Tool::default().execution(json!({
            "url": "https://example.com",
            "bogus": true
        }));

        assert!(matches!(&err, Err(ToolError::UserFacing(_))));
        assert!(err.unwrap_err().to_string().contains("Unknown parameter"));
    }

    #[test]
    fn test_execution_accepts_conditional_fetch_arguments() {
        let ok = Tool::default().execution(json!({
            "url": "https://example.com",
            "if_none_match": "\"abc\"",
            "if_modified_since": "Wed, 21 Oct 2015 07:28:00 GMT"
        }));
        assert!(ok.is_ok());
    }

    #[test]
    fn test_execution_rejects_invalid_url_before_running() {
        let err = Tool::default().execution(json!({"url": "ftp://example.com"}));
        assert!(matches!(&err, Err(ToolError::UserFacing(_))));
        assert!(err
            .unwrap_err()
            .to_string()
            .contains("URL scheme must be http or https"));
    }

    #[test]
    fn test_tool_status() {
        let status = ToolStatus::new("fetch")
            .with_message("Fetching URL")
            .with_percent(50.0)
            .with_eta(5000);

        assert_eq!(status.phase, "fetch");
        assert_eq!(status.message, Some("Fetching URL".to_string()));
        assert_eq!(status.percent_complete, Some(50.0));
        assert_eq!(status.eta_ms, Some(5000));
    }

    #[tokio::test]
    async fn test_build_service_propagates_validation_errors() {
        let mut service = Tool::builder().build_service();
        let err = service.call(json!(["not-an-object"])).await.unwrap_err();
        assert!(err.is_user_facing());
    }
}
