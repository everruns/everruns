// Domain command infrastructure.
//
// The Command trait, CommandError, CommandContext (Ctx), and inventory-based
// dispatch. See knowledge/foundations/domains.md for the full pattern spec.

use crate::storage::StorageBackend;
use axum::Json;
use axum::http::StatusCode;
#[cfg(test)]
use everruns_core::DefaultPermissionResolver;
use everruns_core::{
    Caller, DriverRegistry, EgressService, PermissionResolver, Policy, PolicyError,
};
use everruns_durable::WorkflowEventStore;
use everruns_platform::FeatureFlags;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};
use utoipa::ToSchema;

use crate::api::common::{AllowedAction, ErrorResponse};

// ============================================================================
// CommandError — protocol-agnostic, adapters map to HTTP status / MCP string
// ============================================================================

/// Stable, lower-snake-case category for a command failure. The token set is
/// part of the public MCP contract (see `knowledge/foundations/domains.md` "Structured
/// dispatch errors"); do not rename or drop tokens without a spec update.
#[derive(Debug, thiserror::Error)]
pub enum CommandErrorKind {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unprocessable(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    RateLimited(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// Command failure with optional agent-actionable extensions.
///
/// Carries the `kind`/`message` pair plus the RFC 9457 Problem Details
/// extensions (`code`, `allowed_actions`, `retry_after_seconds`). The HTTP
/// adapter (`From<CommandError> for (StatusCode, Json<ErrorResponse>)`)
/// propagates every extension. MCP `execute` keeps emitting
/// `<kind>: <message>` for bashkit compatibility; surfacing extensions over
/// MCP is a planned additive extension (see `knowledge/foundations/domains.md`).
#[derive(Debug)]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub code: Option<String>,
    pub allowed_actions: Vec<AllowedAction>,
    pub retry_after_seconds: Option<u32>,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.kind.fmt(f)
    }
}

impl std::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

impl From<CommandErrorKind> for CommandError {
    fn from(kind: CommandErrorKind) -> Self {
        Self {
            kind,
            code: None,
            allowed_actions: Vec::new(),
            retry_after_seconds: None,
        }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(e: anyhow::Error) -> Self {
        CommandErrorKind::Internal(e).into()
    }
}

impl CommandError {
    pub fn status(&self) -> StatusCode {
        match &self.kind {
            CommandErrorKind::BadRequest(_) => StatusCode::BAD_REQUEST,
            CommandErrorKind::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            CommandErrorKind::Forbidden(_) => StatusCode::FORBIDDEN,
            CommandErrorKind::NotFound(_) => StatusCode::NOT_FOUND,
            CommandErrorKind::Conflict(_) => StatusCode::CONFLICT,
            CommandErrorKind::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            CommandErrorKind::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Plain message for transport adapters (MCP `<kind>: <message>` etc.).
    ///
    /// For `Internal` this currently returns the underlying anyhow message,
    /// which MAY contain sensitive context (provider hints, stack-trace tail,
    /// SQL fragments). Transport adapters that emit responses to untrusted
    /// callers (HTTP body, public MCP, etc.) MUST redact the `Internal`
    /// variant themselves rather than echoing this value verbatim. The HTTP
    /// adapter (`From<CommandError> for (StatusCode, Json<ErrorResponse>)`)
    /// already does this.
    pub fn message(&self) -> String {
        self.kind.to_string()
    }

    pub fn not_found(resource: &str) -> Self {
        CommandError::not_found_msg(format!("{resource} not found"))
    }

    /// Build a NotFound with the supplied message verbatim. Use when the
    /// "<resource> not found" template isn't a fit (e.g. dispatch told us
    /// "Unknown command: foo").
    pub fn not_found_msg(message: impl Into<String>) -> Self {
        CommandErrorKind::NotFound(message.into()).into()
    }

    pub fn feature_not_enabled(flag: &str) -> Self {
        Self::not_found_msg(format!("Feature '{flag}' is not enabled"))
            .with_code("feature_not_enabled")
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        CommandErrorKind::BadRequest(msg.into()).into()
    }

    pub fn unprocessable(msg: impl Into<String>) -> Self {
        CommandErrorKind::Unprocessable(msg.into()).into()
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        CommandErrorKind::Conflict(msg.into()).into()
    }

    pub fn rate_limited(msg: impl Into<String>) -> Self {
        CommandErrorKind::RateLimited(msg.into()).into()
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        CommandErrorKind::Forbidden(msg.into()).into()
    }

    pub fn internal(err: anyhow::Error) -> Self {
        CommandErrorKind::Internal(err).into()
    }

    // ---- Agent-actionable extension builders ------------------------------

    /// Attach a stable, machine-readable error code (snake_case).
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Attach a recovery action the caller can take.
    pub fn with_action(mut self, action: AllowedAction) -> Self {
        self.allowed_actions.push(action);
        self
    }

    /// Attach a retry hint (seconds). Only meaningful on 429 / transient 503.
    pub fn with_retry_after(mut self, seconds: u32) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }
}

/// Convert anyhow::Error to CommandError, classifying known error patterns.
pub fn classify_anyhow(e: anyhow::Error) -> CommandError {
    // PolicyError → Forbidden
    if let Some(pe) = e.downcast_ref::<PolicyError>() {
        return CommandError::forbidden(pe.message.clone());
    }

    // BadRequestError → BadRequest
    if let Some(br) = e.downcast_ref::<crate::errors::BadRequestError>() {
        return CommandError::bad_request(br.message().to_string());
    }

    // ResourceLimitError → Conflict
    if let Some(limit) = e.downcast_ref::<crate::errors::ResourceLimitError>() {
        return CommandError::conflict(limit.message().to_string());
    }

    // ResourceNotFoundError → NotFound
    if let Some(nf) = e.downcast_ref::<crate::errors::ResourceNotFoundError>() {
        return CommandError::not_found(nf.resource());
    }

    let msg = e.to_string();
    let lowered = msg.to_ascii_lowercase();

    if lowered.contains("duplicate key") || lowered.contains("already exists") {
        return CommandError::conflict(msg).with_code("already_exists");
    }

    // EVE-437: this list catches `anyhow::bail!` strings emitted from
    // `crates/server/src/domains/**` that represent client-input or
    // already-validated resource state, not internal invariants. New entries
    // here keep older calls that have not been migrated to typed errors from
    // silently mapping to 500. New code SHOULD emit `BadRequestError`,
    // `ResourceNotFoundError`, or `PolicyError` directly so this list does
    // not have to grow indefinitely. Each substring is matched against the
    // lowercased anyhow message — keep the patterns short and unambiguous.
    let is_bad_request = [
        "cannot be assigned",
        "cannot be edited",
        "must be archived before deletion",
        "cannot delete built-in",
        "cannot modify built-in",
        "cannot publish archived",
        "cannot unpublish archived",
        "cannot update archived",
        "cannot archive built-in",
        "invalid mcp capability reference",
        "invalid skill capability reference",
        "unsupported locale",
        "unsupported timezone",
        // EVE-437: harness validation
        "harness inheritance cycle detected",
        "harness cannot inherit from itself",
        "parent harness must be active",
        "cannot archive or delete harness while child harnesses",
        // EVE-437: unique-name allocation in agents/harnesses queries
        "could not find a unique name",
        // EVE-437: eval target validation
        "eval target must specify",
        "app targets not yet supported",
        "harness_id and harness_name are mutually exclusive",
        // EVE-437: MCP server config validation (auth-mode and key checks)
        "only api key mcp servers can store an api key",
        "api key auth mode requires",
        "oauth mcp servers require a user connection",
        // EVE-649: org MCP servers reject stdio/local transports at
        // create/update — client-supplied config, not an internal invariant.
        "stdio mcp servers are not supported for organization mcp servers",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern));

    if is_bad_request {
        return CommandError::bad_request(msg);
    }

    CommandError::internal(e)
}

// ============================================================================
// HTTP error conversion
// ============================================================================

impl From<CommandError> for (StatusCode, Json<ErrorResponse>) {
    fn from(e: CommandError) -> Self {
        let status = e.status();
        let CommandError {
            kind,
            code,
            allowed_actions,
            retry_after_seconds,
        } = e;
        let mut body = match kind {
            CommandErrorKind::Internal(inner) => {
                tracing::error!("Command error: {inner}");
                ErrorResponse::new("Internal server error")
                    .with_code(code.unwrap_or_else(|| "internal_error".to_string()))
            }
            other => {
                let mut body = ErrorResponse::new(other.to_string());
                if let Some(c) = code {
                    body = body.with_code(c);
                }
                body
            }
        };
        for action in allowed_actions {
            body = body.with_action(action);
        }
        if let Some(secs) = retry_after_seconds {
            body = body.with_retry_after(secs);
        }
        body.into_response(status)
    }
}

// ============================================================================
// CommandMeta — static metadata for catalog generation
// ============================================================================

#[derive(Debug, Clone)]
pub struct CommandMeta {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub method: &'static str,
    pub path: &'static str,
}

impl CommandMeta {
    /// Feature flag required for this operation to exist in an organization's
    /// API-derived command surfaces.
    pub fn required_feature(&self) -> Option<&'static str> {
        match self.category {
            "evals" => Some("evals"),
            "skills" => Some("skills"),
            "memories" => Some("memory"),
            "knowledge_indexes" | "knowledge_bases" => Some("knowledge"),
            "plugins" => Some("plugins"),
            "observers" => Some("observers"),
            "notifications" => Some("notifications"),
            "payments" => Some("machine_payments"),
            _ => match self.name {
                "list_app_budgets" => Some("app_budgets"),
                "list_agent_versions"
                | "create_agent_version"
                | "set_default_agent_version"
                | "rollback_agent_version"
                | "diff_agent_versions"
                | "fork_agent_version" => Some("agent_versions"),
                _ => None,
            },
        }
    }

    pub fn is_enabled(&self, feature_flags: &FeatureFlags) -> bool {
        self.required_feature()
            .is_none_or(|flag| feature_flags.is_enabled(flag))
    }
}

// ============================================================================
// Ctx — shared execution context
// ============================================================================

static DEFAULT_DRIVER_REGISTRY: LazyLock<Arc<DriverRegistry>> =
    LazyLock::new(|| Arc::new(everruns_worker::create_driver_registry()));

#[cfg(test)]
pub(crate) fn all_feature_flags_for_test() -> FeatureFlags {
    FeatureFlags {
        notifications: true,
        evals: true,
        skills: true,
        memory: true,
        knowledge: true,
        plugins: true,
        app_budgets: true,
        agent_versions: true,
        voice: true,
        agent_delegation: true,
        observers: true,
        public_chat: true,
        webmcp: true,
        machine_payments: true,
    }
}

#[derive(Clone)]
pub struct Ctx {
    pub caller: Caller,
    // SECURITY: Threaded through every caller (HTTP/MCP/gRPC/platform) so
    // `Command::run` evaluates policies against the active (possibly
    // SaaS-custom) resolver. Never bypass via `Policy::evaluate` — that
    // hardcodes `DefaultPermissionResolver`.
    pub permission_resolver: Arc<dyn PermissionResolver>,
    pub db: Arc<StorageBackend>,
    /// Platform driver declarations used by domain validation at provider/model
    /// trust boundaries.
    pub driver_registry: Arc<DriverRegistry>,
    /// Connection providers available to the deployment. Optional because most
    /// command contexts do not inspect connector discovery.
    pub connector_registry: Option<everruns_platform::connector::ConnectorRegistry>,
    pub feature_flags: FeatureFlags,
    pub capability_service: Arc<crate::services::CapabilityService>,
    pub encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    /// Cross-transport resource-creation throttles. Commands that enforce a
    /// throttle use this shared instance so HTTP, MCP, and other adapters
    /// consume the same bucket.
    pub org_rate_limiter: Option<crate::auth::rate_limit::OrgRateLimiter>,
    pub session_service: Option<Arc<crate::domains::sessions::SessionService>>,
    pub message_service: Option<Arc<crate::domains::messages::MessageService>>,
    pub event_service: Option<Arc<crate::services::EventService>>,
    pub session_file_service: Option<Arc<crate::domains::session_files::WorkspaceFileService>>,
    pub session_sandbox_service:
        Option<Arc<crate::domains::session_sandbox::SessionSandboxService>>,
    pub session_schedule_service:
        Option<Arc<crate::domains::session_schedules::SessionScheduleService>>,
    pub notification_service: Option<Arc<crate::domains::notifications::NotificationService>>,
    pub model_service: Option<Arc<crate::domains::models::ModelService>>,
    pub provider_service: Option<Arc<crate::domains::providers::ProviderService>>,
    pub model_sync_service: Option<Arc<crate::services::ModelSyncService>>,
    pub eval_service: Option<Arc<crate::domains::evals::EvalService>>,
    pub reporting_service: Option<Arc<crate::domains::reporting::ReportingService>>,
    pub sqldb_store: Option<Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore>>,
    pub workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    pub runner: Option<Arc<dyn everruns_worker::AgentRunner>>,
    pub fallback_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
    /// Outbound HTTP boundary. Used by commands that make sanctioned egress
    /// calls (e.g. plugin sync/fetch from GitHub or a URL source).
    pub egress_service: Option<Arc<dyn EgressService>>,
    /// System utility LLM for sanctioned internal analysis tasks
    /// (knowledge/operations/utility-llm.md). Not a user-configurable model surface.
    pub utility_llm_service: Option<Arc<dyn everruns_core::UtilityLlmService>>,
    /// Agent health check service (knowledge/evaluation/agent-checks.md, tier-3).
    pub health_check_service: Option<Arc<crate::domains::agents::AgentHealthCheckService>>,
    /// Per-org/per-user resource caps enforced in create paths (harnesses,
    /// agents, sessions). Resolved from env so SaaS plan overrides apply across
    /// every entry path (HTTP/MCP/gRPC). Tests may override the fields directly.
    pub resource_limits: crate::server::ResourceLimitsConfig,
}

impl Ctx {
    pub fn org_id(&self) -> i64 {
        self.caller.org_id
    }

    /// Construct a Ctx for an HTTP request.
    ///
    /// `encryption` may be `None` for domains that never need it (agents,
    /// harnesses, skills). Domains that encrypt secrets (apps, mcp_servers)
    /// must pass `Some`.
    ///
    /// `permission_resolver` is required — pass `auth.permission_resolver`
    /// from `AuthState` so SaaS-custom resolvers are honored during
    /// enforcement. In tests that don't care about the resolver, use
    /// `Ctx::minimal_for_test`; use `with_permission_resolver` when a test
    /// needs to override the resolver explicitly.
    pub fn new(
        caller: Caller,
        db: Arc<StorageBackend>,
        capability_service: Arc<crate::services::CapabilityService>,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
        permission_resolver: Arc<dyn PermissionResolver>,
    ) -> Self {
        Self {
            caller,
            permission_resolver,
            db,
            driver_registry: DEFAULT_DRIVER_REGISTRY.clone(),
            connector_registry: None,
            feature_flags: FeatureFlags::current(),
            capability_service,
            encryption,
            org_rate_limiter: None,
            session_service: None,
            message_service: None,
            event_service: None,
            session_file_service: None,
            session_sandbox_service: None,
            session_schedule_service: None,
            notification_service: None,
            model_service: None,
            provider_service: None,
            model_sync_service: None,
            eval_service: None,
            reporting_service: None,
            sqldb_store: None,
            workflow_store: None,
            runner: None,
            fallback_harness_name: None,
            chat_harness_name: None,
            chat_session_title: None,
            egress_service: None,
            utility_llm_service: None,
            health_check_service: None,
            resource_limits: crate::server::ResourceLimitsConfig::from_env(),
        }
    }

    pub fn minimal(
        caller: Caller,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
        permission_resolver: Arc<dyn PermissionResolver>,
    ) -> Self {
        let capability_service =
            Arc::new(crate::services::CapabilityService::new(db.clone(), None));
        Self::new(
            caller,
            db,
            capability_service,
            encryption,
            permission_resolver,
        )
    }

    /// Test-only convenience: construct a minimal Ctx with the OSS default
    /// resolver and all API-visible features enabled. Tests for disabled
    /// behavior must override `feature_flags` explicitly. Production code must never use this — route
    /// `AuthState.permission_resolver` through `Ctx::new` instead so SaaS
    /// overrides take effect.
    #[cfg(test)]
    pub fn minimal_for_test(
        caller: Caller,
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::encryption::EncryptionService>>,
    ) -> Self {
        Self::minimal(caller, db, encryption, Arc::new(DefaultPermissionResolver))
            .with_feature_flags(all_feature_flags_for_test())
    }

    /// Override the permission resolver. Primarily for tests; production
    /// code should pass the resolver to `Ctx::new` directly.
    pub fn with_permission_resolver(mut self, resolver: Arc<dyn PermissionResolver>) -> Self {
        self.permission_resolver = resolver;
        self
    }

    pub fn with_feature_flags(mut self, feature_flags: FeatureFlags) -> Self {
        self.feature_flags = feature_flags;
        self
    }

    pub fn with_driver_registry(mut self, driver_registry: Arc<DriverRegistry>) -> Self {
        self.driver_registry = driver_registry;
        self
    }

    pub fn with_connector_registry(
        mut self,
        connector_registry: everruns_platform::connector::ConnectorRegistry,
    ) -> Self {
        self.connector_registry = Some(connector_registry);
        self
    }

    pub fn with_org_rate_limiter(
        mut self,
        limiter: crate::auth::rate_limit::OrgRateLimiter,
    ) -> Self {
        self.org_rate_limiter = Some(limiter);
        self
    }

    pub fn with_session_service(
        mut self,
        service: Arc<crate::domains::sessions::SessionService>,
    ) -> Self {
        self.session_service = Some(service);
        self
    }

    pub fn with_message_service(
        mut self,
        service: Arc<crate::domains::messages::MessageService>,
    ) -> Self {
        self.message_service = Some(service);
        self
    }

    pub fn with_event_service(mut self, service: Arc<crate::services::EventService>) -> Self {
        self.event_service = Some(service);
        self
    }

    pub fn with_session_file_service(
        mut self,
        service: Arc<crate::domains::session_files::WorkspaceFileService>,
    ) -> Self {
        self.session_file_service = Some(service);
        self
    }

    pub fn with_session_sandbox_service(
        mut self,
        service: Arc<crate::domains::session_sandbox::SessionSandboxService>,
    ) -> Self {
        self.session_sandbox_service = Some(service);
        self
    }

    pub fn with_session_schedule_service(
        mut self,
        service: Arc<crate::domains::session_schedules::SessionScheduleService>,
    ) -> Self {
        self.session_schedule_service = Some(service);
        self
    }

    pub fn with_notification_service(
        mut self,
        service: Arc<crate::domains::notifications::NotificationService>,
    ) -> Self {
        self.notification_service = Some(service);
        self
    }

    pub fn with_model_service(
        mut self,
        service: Arc<crate::domains::models::ModelService>,
    ) -> Self {
        self.model_service = Some(service);
        self
    }

    pub fn with_provider_service(
        mut self,
        service: Arc<crate::domains::providers::ProviderService>,
    ) -> Self {
        self.provider_service = Some(service);
        self
    }

    pub fn with_model_sync_service(
        mut self,
        service: Arc<crate::services::ModelSyncService>,
    ) -> Self {
        self.model_sync_service = Some(service);
        self
    }

    pub fn with_eval_service(mut self, service: Arc<crate::domains::evals::EvalService>) -> Self {
        self.eval_service = Some(service);
        self
    }

    pub fn with_reporting_service(
        mut self,
        service: Arc<crate::domains::reporting::ReportingService>,
    ) -> Self {
        self.reporting_service = Some(service);
        self
    }

    pub fn with_sqldb_store(
        mut self,
        store: Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore>,
    ) -> Self {
        self.sqldb_store = Some(store);
        self
    }

    pub fn with_workflow_store(
        mut self,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    ) -> Self {
        self.workflow_store = workflow_store;
        self
    }

    pub fn with_runner(mut self, runner: Arc<dyn everruns_worker::AgentRunner>) -> Self {
        self.runner = Some(runner);
        self
    }

    pub fn with_utility_llm_service(
        mut self,
        service: Arc<dyn everruns_core::UtilityLlmService>,
    ) -> Self {
        self.utility_llm_service = Some(service);
        self
    }

    pub fn with_health_check_service(
        mut self,
        service: Arc<crate::domains::agents::AgentHealthCheckService>,
    ) -> Self {
        self.health_check_service = Some(service);
        self
    }

    pub fn with_fallback_harness_name(mut self, name: Option<String>) -> Self {
        self.fallback_harness_name = name;
        self
    }

    pub fn with_chat_harness_name(mut self, name: Option<String>) -> Self {
        self.chat_harness_name = name;
        self
    }

    pub fn with_chat_session_title(mut self, title: Option<String>) -> Self {
        self.chat_session_title = title;
        self
    }

    pub fn with_egress_service(mut self, service: Arc<dyn EgressService>) -> Self {
        self.egress_service = Some(service);
        self
    }
}

// ============================================================================
// Command trait
// ============================================================================

pub trait Command: DeserializeOwned + Send + 'static + CommandSchema {
    type Output: Serialize + Send;

    /// Static metadata — drives MCP catalog generation.
    fn meta() -> CommandMeta;

    /// Whether this command is read-only when exposed through scripted MCP tools.
    ///
    /// Defaults to GET-only. Override for read-only POST-style helpers such as
    /// previews or search operations that accept structured bodies.
    fn read_only() -> bool {
        matches!(Self::meta().method, "GET")
    }

    /// Policy to check before execution. None = public/no auth.
    fn policy() -> Option<&'static Policy> {
        None
    }

    /// If set, allows MCP `execute` callers to pass the value for this field as
    /// a single positional argument (e.g. `get_agent <id>` instead of
    /// `get_agent --id <id>`). bashkit's flag parser hardcodes `expected --flag`
    /// for positional args, so the command string is pre-rewritten to insert
    /// `--<positional_arg>` before the value. See EVE-323.
    fn positional_arg() -> Option<&'static str> {
        None
    }

    /// JSON Schema for the command input surfaced in the MCP catalog.
    fn param_schema() -> Value {
        json_schema_for_command::<Self>()
    }

    /// JSON Schema for the command output surfaced in MCP discovery.
    ///
    /// Commands should override this when the output shape is important for
    /// scripting. The default remains a valid open schema so discovery can
    /// always report an output contract without blocking command migration.
    fn output_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": true,
            "description": "Output schema is not declared for this command yet."
        })
    }

    /// Short jq-oriented output shape hint for MCP scripting callers.
    fn output_shape() -> &'static str {
        "unknown"
    }

    /// Execute the command. Validation + business logic + persistence.
    ///
    /// SECURITY: Do not call this directly from HTTP/MCP/gRPC adapters —
    /// it skips the policy check. Use `Command::run` instead. `execute`
    /// stays public to allow one command to compose another (already
    /// authorized through its own `run`) without re-paying policy cost.
    fn execute(self, ctx: &Ctx) -> impl Future<Output = Result<Self::Output, CommandError>> + Send;

    /// Public entry point. Evaluates `policy()` against the active
    /// `PermissionResolver` (from `Ctx`) and then delegates to `execute`.
    ///
    /// Every caller — HTTP adapters, MCP dispatch, gRPC `ExecuteCommand`,
    /// platform capability — goes through this. If you're adding a new
    /// caller and want to skip policy, stop: write a new command.
    ///
    /// This is also the protocol-agnostic chokepoint for cross-cutting
    /// concerns: a tracing span per command, a counter and a duration
    /// histogram with `name` / `category` / `status` labels, and a debug-level
    /// log on failure. HTTP-specific concerns (request_id propagation,
    /// content-type negotiation, response body decoration) live in the HTTP
    /// `Dispatcher`, which sits between the route and this method.
    fn run(self, ctx: &Ctx) -> impl Future<Output = Result<Self::Output, CommandError>> + Send
    where
        Self: Sized,
    {
        use tracing::Instrument;

        let meta = Self::meta();
        let span = tracing::info_span!(
            "command",
            "command.name" = meta.name,
            "command.category" = meta.category,
            "command.method" = meta.method,
            "command.path" = meta.path,
            "org.id" = ctx.caller.org_id,
            "caller.internal" = ctx.caller.is_internal,
        );

        async move {
            let started_at = std::time::Instant::now();

            let result: Result<Self::Output, CommandError> = async {
                if let Some(flag) = meta.required_feature()
                    && !ctx.feature_flags.is_enabled(flag)
                {
                    return Err(CommandError::feature_not_enabled(flag));
                }
                if let Some(policy) = Self::policy() {
                    policy
                        .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                        .map_err(|e| CommandError::forbidden(e.message))?;
                }
                self.execute(ctx).await
            }
            .await;

            let elapsed_secs = started_at.elapsed().as_secs_f64();
            let status = match &result {
                Ok(_) => "ok",
                Err(err) => command_error_status_label(err),
            };

            metrics::counter!(
                crate::api::prometheus::names::COMMANDS_TOTAL,
                "name" => meta.name,
                "category" => meta.category,
                "status" => status,
            )
            .increment(1);
            metrics::histogram!(
                crate::api::prometheus::names::COMMAND_DURATION,
                "name" => meta.name,
                "category" => meta.category,
                "status" => status,
            )
            .record(elapsed_secs);

            if let Err(err) = &result {
                // 5xx (Internal) at error! so MCP/gRPC failures surface in
                // production logs even though those protocols don't run the
                // HTTP `From<CommandError>` converter that historically logged
                // them. 4xx remain at debug! — they're caller-induced and
                // would dominate log volume otherwise.
                match err {
                    CommandError {
                        kind: CommandErrorKind::Internal(_),
                        ..
                    } => {
                        tracing::error!(status, error = %err, "command failed");
                    }
                    _ => {
                        tracing::debug!(status, error = %err, "command failed");
                    }
                }
            }

            result
        }
        .instrument(span)
    }
}

/// Stable, low-cardinality status label for the `everruns_commands_total`
/// counter and `everruns_command_duration_seconds` histogram.
fn command_error_status_label(err: &CommandError) -> &'static str {
    match err {
        CommandError {
            kind: CommandErrorKind::BadRequest(_),
            ..
        } => "bad_request",
        CommandError {
            kind: CommandErrorKind::Unprocessable(_),
            ..
        } => "unprocessable",
        CommandError {
            kind: CommandErrorKind::Forbidden(_),
            ..
        } => "forbidden",
        CommandError {
            kind: CommandErrorKind::NotFound(_),
            ..
        } => "not_found",
        CommandError {
            kind: CommandErrorKind::Conflict(_),
            ..
        } => "conflict",
        CommandError {
            kind: CommandErrorKind::RateLimited(_),
            ..
        } => "rate_limited",
        CommandError {
            kind: CommandErrorKind::Internal(_),
            ..
        } => "internal",
    }
}

pub trait CommandSchema {
    fn param_schema() -> Value;
}

impl<T> CommandSchema for T
where
    T: ToSchema,
{
    fn param_schema() -> Value {
        json_schema_for::<T>()
    }
}

fn json_schema_for_command<T>() -> Value
where
    T: Command + CommandSchema,
{
    <T as CommandSchema>::param_schema()
}

pub fn delegated_param_schema<T>() -> Value
where
    T: ToSchema,
{
    json_schema_for::<T>()
}

pub fn output_schema_for<T>() -> Value
where
    T: ToSchema,
{
    json_schema_for::<T>()
}

pub fn array_output_schema(item_schema: Value) -> Value {
    serde_json::json!({
        "type": "array",
        "items": item_schema
    })
}

pub fn paginated_output_schema(item_schema: Value) -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "data": {
                "type": "array",
                "items": item_schema
            },
            "total": { "type": "integer" },
            "offset": { "type": "integer" },
            "limit": { "type": "integer" }
        },
        "required": ["data", "total", "offset", "limit"]
    })
}

fn json_schema_for<T>() -> Value
where
    T: ToSchema,
{
    let mut schema = match serde_json::to_value(T::schema()) {
        Ok(schema) => schema,
        Err(err) => {
            tracing::warn!(
                schema_type = std::any::type_name::<T>(),
                error = %err,
                "failed to serialize command schema; falling back to open object schema"
            );
            return open_object_schema();
        }
    };

    rewrite_schema_refs(&mut schema);

    let mut defs = serde_json::Map::new();
    let mut refs = Vec::new();
    T::schemas(&mut refs);
    for (name, ref_or_schema) in refs {
        let mut value = match serde_json::to_value(ref_or_schema) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(
                    schema_type = std::any::type_name::<T>(),
                    schema_name = %name,
                    error = %err,
                    "failed to serialize referenced schema; using open object schema in $defs"
                );
                open_object_schema()
            }
        };
        rewrite_schema_refs(&mut value);
        defs.insert(name, value);
    }

    if !defs.is_empty()
        && let Some(obj) = schema.as_object_mut()
    {
        obj.insert("$defs".to_string(), Value::Object(defs));
    }

    schema
}

fn open_object_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": true,
    })
}

fn rewrite_schema_refs(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref")
                && let Some(name) = reference.strip_prefix("#/components/schemas/")
            {
                *reference = format!("#/$defs/{name}");
            }

            for child in map.values_mut() {
                rewrite_schema_refs(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_schema_refs(item);
            }
        }
        _ => {}
    }
}

// ============================================================================
// Paginated — generic list response
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct Paginated<T: Serialize> {
    pub data: Vec<T>,
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
}

// ============================================================================
// CommandDescriptor — type-erased entry for inventory
// ============================================================================

type DispatchFn =
    for<'a> fn(
        serde_json::Value,
        &'a Ctx,
    ) -> Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send + 'a>>;

pub struct CommandDescriptor {
    pub meta: fn() -> CommandMeta,
    pub read_only: fn() -> bool,
    pub positional_arg: fn() -> Option<&'static str>,
    pub param_schema: fn() -> Value,
    pub output_schema: fn() -> Value,
    pub output_shape: fn() -> &'static str,
    // SECURITY: Exposed so the policy-coverage test can assert every
    // mutating command declares a policy; do not drop.
    pub policy: fn() -> Option<&'static Policy>,
    pub dispatch: DispatchFn,
}

inventory::collect!(CommandDescriptor);

impl CommandDescriptor {
    /// Create a descriptor for a Command impl. Use with `inventory::submit!`.
    pub const fn of<C: Command>() -> Self {
        Self {
            meta: C::meta,
            read_only: C::read_only,
            positional_arg: C::positional_arg,
            param_schema: <C as Command>::param_schema,
            output_schema: C::output_schema,
            output_shape: C::output_shape,
            policy: C::policy,
            dispatch: dispatch_for::<C>,
        }
    }
}

fn dispatch_for<C: Command>(
    params: serde_json::Value,
    ctx: &Ctx,
) -> Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send + '_>> {
    Box::pin(async move {
        // Command transports encode no arguments as `{}`, while serde represents
        // Rust unit structs as `null`. Retry only the empty-object shape so
        // commands with declared fields keep their original validation errors.
        let empty_object = params.as_object().is_some_and(serde_json::Map::is_empty);
        let cmd: C = match serde_json::from_value(params) {
            Ok(cmd) => cmd,
            Err(object_error) if empty_object => serde_json::from_value(serde_json::Value::Null)
                .map_err(|_| CommandError::bad_request(object_error.to_string()))?,
            Err(error) => return Err(CommandError::bad_request(error.to_string())),
        };

        // `run` handles the policy check using the active resolver in `ctx`.
        let result = cmd.run(ctx).await?;

        serde_json::to_string(&result).map_err(|e| CommandError::internal(e.into()))
    })
}

// ============================================================================
// Dispatch table — built once, used by MCP endpoint
// ============================================================================

use std::collections::HashMap;

static DISPATCH_TABLE: LazyLock<HashMap<&'static str, &'static CommandDescriptor>> =
    LazyLock::new(|| {
        inventory::iter::<CommandDescriptor>
            .into_iter()
            .map(|desc| ((desc.meta)().name, desc))
            .collect()
    });

/// Dispatch a command by name from JSON params. Used by MCP endpoint.
pub async fn dispatch(
    name: &str,
    params: serde_json::Value,
    ctx: &Ctx,
) -> Result<String, CommandError> {
    let desc = DISPATCH_TABLE
        .get(name)
        .ok_or_else(|| CommandError::not_found_msg(format!("Unknown command: {name}")))?;
    (desc.dispatch)(params, ctx).await
}

/// Build catalog entries from all registered commands.
pub fn catalog_entries() -> Vec<CommandMeta> {
    inventory::iter::<CommandDescriptor>
        .into_iter()
        .map(|desc| (desc.meta)())
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandCatalogEntry {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub read_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub positional_arg: Option<&'static str>,
    pub input_schema: Value,
    pub output_schema: Value,
    pub output_shape: &'static str,
}

pub fn catalog_entries_with_schemas(
    include_schemas: bool,
    feature_flags: &FeatureFlags,
) -> Vec<CommandCatalogEntry> {
    inventory::iter::<CommandDescriptor>
        .into_iter()
        .filter(|desc| (desc.meta)().is_enabled(feature_flags))
        .map(|desc| {
            let meta = (desc.meta)();
            CommandCatalogEntry {
                name: meta.name,
                category: meta.category,
                description: meta.description,
                method: meta.method,
                path: meta.path,
                read_only: (desc.read_only)(),
                positional_arg: (desc.positional_arg)(),
                input_schema: if include_schemas {
                    (desc.param_schema)()
                } else {
                    Value::Null
                },
                output_schema: if include_schemas {
                    (desc.output_schema)()
                } else {
                    Value::Null
                },
                output_shape: (desc.output_shape)(),
            }
        })
        .collect()
}

// ============================================================================
// Validation helpers shared across domains
// ============================================================================

/// Validate an addressable name (agent, harness, etc.)
pub fn validate_name(entity: &str, name: &str) -> Result<(), CommandError> {
    everruns_platform::validate_addressable_name(name)
        .map_err(|msg| CommandError::bad_request(format!("{entity} {msg}")))
}

/// Clamp pagination params to safe defaults.
pub fn pagination(offset: Option<u32>, limit: Option<u32>) -> crate::api::common::Pagination {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(20).min(100);
    crate::api::common::Pagination::new(offset, limit)
}

// ============================================================================
// Lenient deserializers — accept both typed JSON values and stringly-typed
// values from the MCP CLI bridge (bashkit catalog).
// ============================================================================
//
// Context (EVE-324): inventory-registered commands expose an open JSON schema
// (`additionalProperties: true`) to bashkit because the request types own
// their own serde shape. Without per-flag type hints, bashkit's flag parser
// defaults every value to a string — so `list_capabilities --limit 5` arrives
// at dispatch as `{"limit": "5"}`. Serde then rejects that string while
// trying to populate `Option<u32>`, the dispatcher wraps the error as
// `CommandError::BadRequest`, and the bashkit adapter previously sanitized it
// into the opaque `<cmd>: callback failed` the caller saw. Today the dispatch
// layer emits `<kind>: <message>` (see `format_dispatch_error` in
// `crates/server/src/api/mcp_endpoint/catalog.rs`), and bashkit prepends the
// builtin name on its own, so the combined wire output is
// `<cmd>: <kind>: <message>` and callers can tell a `bad_request` from an
// `internal` failure — but the lenient helpers below still spare us a class
// of `bad_request: invalid type: string …` surprises.
//
// The helpers below sit between the serde field and the raw JSON: they accept
// the native typed shape (for programmatic callers) and also coerce the
// string forms bashkit produces. They are intentionally scoped to the input
// fields that are known to arrive from the flag parser — primarily pagination
// (`offset`, `limit`) and boolean toggles (`include_archived`).
/// Accept `Option<u32>` as `null`, an integer, or a numeric string.
pub fn deserialize_opt_u32_lenient<'de, D>(d: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Num(u32),
        Str(String),
    }

    match Option::<Either>::deserialize(d)? {
        None => Ok(None),
        Some(Either::Num(n)) => Ok(Some(n)),
        Some(Either::Str(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<u32>()
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }
}

/// Accept `Option<serde_json::Value>` as a native JSON value, `null`, or a JSON string.
pub fn deserialize_opt_json_value_lenient<'de, D>(d: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Str(String),
        Value(Value),
    }

    match Option::<Either>::deserialize(d)? {
        None => Ok(None),
        Some(Either::Value(value)) => Ok(Some(value)),
        Some(Either::Str(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            serde_json::from_str::<Value>(trimmed)
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }
}

/// Accept `bool`, `"true"`/`"false"`/`"1"`/`"0"`/`"yes"`/`"no"` (case-insensitive),
/// or integer `0`/`1`. Missing keys fall through to serde's default handling.
pub fn deserialize_bool_lenient<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Bool(bool),
        Num(i64),
        Str(String),
    }

    match Either::deserialize(d)? {
        Either::Bool(b) => Ok(b),
        Either::Num(0) => Ok(false),
        Either::Num(1) => Ok(true),
        Either::Num(n) => Err(serde::de::Error::custom(format!(
            "cannot coerce integer {n} to bool (expected 0 or 1)"
        ))),
        Either::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Ok(true),
            "false" | "0" | "no" | "n" | "off" => Ok(false),
            other => Err(serde::de::Error::custom(format!(
                "cannot coerce string {other:?} to bool"
            ))),
        },
    }
}

/// Option-aware variant of [`deserialize_bool_lenient`] for `Option<bool>`
/// fields that come in via the MCP/bashkit flag parser, which forwards bools
/// as JSON strings (`"true"`/`"false"`). Present values pass through the
/// lenient bool coercion; explicit `null` becomes `None`.
///
/// Pair with `#[serde(default)]` at the call site so a missing field stays
/// `None`. Without `#[serde(default)]`, serde raises `missing field` before
/// this function runs and the lenient coercion never gets a chance.
pub fn deserialize_opt_bool_lenient<'de, D>(d: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Wrap(#[serde(deserialize_with = "deserialize_bool_lenient")] bool);

    Ok(Option::<Wrap>::deserialize(d)?.map(|Wrap(b)| b))
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn command_metadata_declares_feature_gated_surfaces() {
        for (name, category, expected) in [
            ("list_evals", "evals", Some("evals")),
            ("list_skills", "skills", Some("skills")),
            ("list_memories", "memories", Some("memory")),
            (
                "list_knowledge_indexes",
                "knowledge_indexes",
                Some("knowledge"),
            ),
            ("list_plugins", "plugins", Some("plugins")),
            ("create_observer", "observers", Some("observers")),
            ("list_notifications", "notifications", Some("notifications")),
            (
                "list_payment_accounts",
                "payments",
                Some("machine_payments"),
            ),
            ("list_app_budgets", "budgets", Some("app_budgets")),
            ("create_agent_version", "agents", Some("agent_versions")),
            ("list_agents", "agents", None),
        ] {
            let meta = CommandMeta {
                name,
                category,
                description: "",
                method: "GET",
                path: "",
            };
            assert_eq!(meta.required_feature(), expected, "command {name}");
        }
    }

    #[tokio::test]
    async fn dispatch_accepts_empty_object_for_unit_commands() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = Ctx::minimal_for_test(Caller::internal(1), db, None);

        let result = dispatch("get_report_catalog", serde_json::json!({}), &ctx)
            .await
            .expect("empty MCP params should dispatch to a unit command");
        let value: serde_json::Value = serde_json::from_str(&result).expect("catalog JSON");

        assert!(value.get("datasets").is_some());
    }

    #[tokio::test]
    async fn dispatch_does_not_coerce_nonempty_objects_for_unit_commands() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = Ctx::minimal_for_test(Caller::internal(1), db, None);

        let error = dispatch(
            "get_report_catalog",
            serde_json::json!({ "unexpected": true }),
            &ctx,
        )
        .await
        .expect_err("non-empty params must remain invalid for a unit command");

        assert!(matches!(error.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn classify_anyhow_maps_bad_request_error() {
        let err = classify_anyhow(crate::errors::BadRequestError::new("bad input").into());
        assert!(
            matches!(err, CommandError { kind: CommandErrorKind::BadRequest(msg), .. } if msg == "bad input")
        );
    }

    // Cardinality contract for the `status` label on `everruns_commands_total`
    // / `everruns_command_duration_seconds`. These strings are an external
    // surface — Prometheus dashboards and alerts grep on them — so they must
    // stay stable. Adding a CommandError variant requires a label here.
    #[test]
    fn command_error_status_label_covers_every_variant() {
        assert_eq!(
            command_error_status_label(&CommandError::bad_request("x")),
            "bad_request"
        );
        assert_eq!(
            command_error_status_label(&CommandError::unprocessable("x")),
            "unprocessable"
        );
        assert_eq!(
            command_error_status_label(&CommandError::forbidden("x")),
            "forbidden"
        );
        assert_eq!(
            command_error_status_label(&CommandError::not_found_msg("x")),
            "not_found"
        );
        assert_eq!(
            command_error_status_label(&CommandError::conflict("x")),
            "conflict"
        );
        assert_eq!(
            command_error_status_label(&CommandError::internal(anyhow::anyhow!("x"))),
            "internal"
        );
    }

    #[test]
    fn classify_anyhow_maps_not_found_error() {
        let err = classify_anyhow(crate::errors::ResourceNotFoundError::new("Thing").into());
        assert!(
            matches!(err, CommandError { kind: CommandErrorKind::NotFound(msg), .. } if msg == "Thing not found")
        );
    }

    // EVE-645: MCP server lookups raise `ResourceNotFoundError::new("MCP
    // server")` (mcp_servers/service.rs) instead of a stringly-typed
    // `anyhow!("MCP server not found")` that fell through to Internal/500.
    // Pin that it now classifies as a 404.
    #[test]
    fn classify_anyhow_maps_mcp_server_not_found() {
        let err = classify_anyhow(crate::errors::ResourceNotFoundError::new("MCP server").into());
        let CommandError {
            kind: CommandErrorKind::NotFound(msg),
            ..
        } = &err
        else {
            panic!("MCP server not-found must classify as NotFound, got {err:?}");
        };
        assert_eq!(msg, "MCP server not found");
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
    }

    // EVE-649: org MCP server create/update raise a stringly-typed
    // `anyhow::bail!("stdio MCP servers are not supported for organization MCP
    // servers")` on client-supplied transport config. Pin that it classifies
    // as a 400 instead of falling through to Internal/500.
    #[test]
    fn classify_anyhow_maps_stdio_mcp_server_rejection_to_bad_request() {
        let err = classify_anyhow(anyhow::anyhow!(
            "stdio MCP servers are not supported for organization MCP servers"
        ));
        let CommandError {
            kind: CommandErrorKind::BadRequest(_),
            ..
        } = &err
        else {
            panic!("stdio MCP server rejection must classify as BadRequest, got {err:?}");
        };
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    // EVE-437: every substring added to the `is_bad_request` list must in
    // fact map an `anyhow::bail!` to `CommandError::BadRequest` rather than
    // silently 500ing. New entries here pin the contract.
    #[test]
    fn classify_anyhow_maps_eve437_validation_substrings() {
        let cases = [
            "Harness inheritance cycle detected",
            "Harness cannot inherit from itself",
            "Parent harness must be active",
            "Cannot archive or delete harness while child harnesses still inherit from it: a, b",
            "Could not find a unique name for 'foo'",
            "eval target must specify harness_id or harness_name",
            "App targets not yet supported in eval execution",
            "harness_id and harness_name are mutually exclusive in eval target",
            "Only API key MCP servers can store an API key",
            "API key auth mode requires an API key",
            "API key auth mode requires a non-empty API key",
            "OAuth MCP servers require a user connection before tools can be refreshed",
            // EVE-649: org MCP server transport validation
            "stdio MCP servers are not supported for organization MCP servers",
        ];
        for raw in cases {
            let err = classify_anyhow(anyhow::anyhow!("{raw}"));
            assert!(
                matches!(
                    err,
                    CommandError {
                        kind: CommandErrorKind::BadRequest(_),
                        ..
                    }
                ),
                "{raw} must classify as BadRequest, got {err:?}"
            );
        }
    }

    // EVE-437: a generic anyhow error with no recognized pattern must keep
    // mapping to Internal so we do not accidentally widen the bad-request
    // class for unrelated future failures.
    #[test]
    fn classify_anyhow_unknown_message_is_internal() {
        let err = classify_anyhow(anyhow::anyhow!("connection timed out"));
        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::Internal(_),
                    ..
                }
            ),
            "unknown messages must remain Internal: {err:?}"
        );
    }

    // EVE-437: PolicyError stays mapped to Forbidden — this is the path used
    // by the new sessions/service.rs high-risk capability check that used to
    // be a bail! and 500'd.
    #[test]
    fn classify_anyhow_maps_policy_error() {
        let pe = everruns_core::PolicyError::denied("test_policy", "admin role");
        let err = classify_anyhow(anyhow::Error::new(pe));
        let CommandError {
            kind: CommandErrorKind::Forbidden(msg),
            ..
        } = &err
        else {
            panic!("PolicyError must classify as Forbidden, got {err:?}");
        };
        assert!(
            msg.contains("admin role"),
            "Forbidden message must include detail, got {msg:?}"
        );
    }

    #[test]
    fn http_adapter_propagates_extensions() {
        use crate::api::common::AllowedAction;
        let err = CommandError::conflict("agent already exists")
            .with_code("agent_already_exists")
            .with_action(
                AllowedAction::new("get-existing")
                    .with_operation_id("get_agent")
                    .with_hint("Fetch the existing agent and reuse it."),
            );
        let (status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);
        assert_eq!(status, StatusCode::CONFLICT);
        let body = body.0;
        assert_eq!(body.status, 409);
        assert_eq!(body.title, "Conflict");
        assert_eq!(body.detail.as_deref(), Some("agent already exists"));
        assert_eq!(body.code.as_deref(), Some("agent_already_exists"));
        assert_eq!(body.allowed_actions.len(), 1);
        assert_eq!(body.allowed_actions[0].rel, "get-existing");
        assert_eq!(
            body.allowed_actions[0].operation_id.as_deref(),
            Some("get_agent")
        );
    }

    #[test]
    fn http_adapter_propagates_retry_after() {
        let err = CommandError::unprocessable("backend warming up").with_retry_after(30);
        let (status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body.0.retry_after_seconds, Some(30));
    }

    #[test]
    fn http_adapter_redacts_internal_detail() {
        let err = CommandError::internal(anyhow::anyhow!("db handle exhausted: secret=YExample0"));
        let (_status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);
        // detail is the safe generic string, never the anyhow message.
        assert_eq!(body.0.detail.as_deref(), Some("Internal server error"));
        assert_eq!(body.0.code.as_deref(), Some("internal_error"));
    }

    #[test]
    fn mcp_format_dispatch_error_unchanged_by_extensions() {
        // Bashkit consumers parse `<kind>: <message>` — adding extensions to
        // CommandError MUST NOT alter that wire string.
        let plain = CommandError::bad_request("missing field 'name'");
        let with_ext = CommandError::bad_request("missing field 'name'")
            .with_code("agent_name_required")
            .with_retry_after(0);
        assert_eq!(plain.to_string(), "missing field 'name'");
        assert_eq!(with_ext.to_string(), "missing field 'name'");
    }
}

#[cfg(test)]
mod lenient_tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct PageInput {
        #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
        offset: Option<u32>,
        #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
        limit: Option<u32>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct FlagInput {
        #[serde(default, deserialize_with = "deserialize_bool_lenient")]
        include_archived: bool,
    }

    #[test]
    fn opt_u32_lenient_accepts_integer() {
        let v: PageInput = serde_json::from_value(json!({"offset": 0, "limit": 5})).unwrap();
        assert_eq!(
            v,
            PageInput {
                offset: Some(0),
                limit: Some(5)
            }
        );
    }

    #[test]
    fn opt_u32_lenient_accepts_numeric_string() {
        let v: PageInput = serde_json::from_value(json!({"offset": "10", "limit": "20"})).unwrap();
        assert_eq!(
            v,
            PageInput {
                offset: Some(10),
                limit: Some(20)
            }
        );
    }

    #[test]
    fn opt_u32_lenient_accepts_missing_and_null() {
        let missing: PageInput = serde_json::from_value(json!({})).unwrap();
        assert_eq!(
            missing,
            PageInput {
                offset: None,
                limit: None
            }
        );
        let nulled: PageInput =
            serde_json::from_value(json!({"offset": null, "limit": null})).unwrap();
        assert_eq!(
            nulled,
            PageInput {
                offset: None,
                limit: None
            }
        );
    }

    #[test]
    fn opt_u32_lenient_rejects_non_numeric_string() {
        let err = serde_json::from_value::<PageInput>(json!({"limit": "abc"})).unwrap_err();
        assert!(err.to_string().contains("invalid digit"), "got: {err}");
    }

    #[test]
    fn bool_lenient_accepts_variants() {
        for (input, expected) in [
            (json!(true), true),
            (json!(false), false),
            (json!("true"), true),
            (json!("FALSE"), false),
            (json!("1"), true),
            (json!("0"), false),
            (json!("yes"), true),
            (json!("no"), false),
            (json!(1), true),
            (json!(0), false),
        ] {
            let v: FlagInput = serde_json::from_value(json!({"include_archived": input})).unwrap();
            assert_eq!(v.include_archived, expected, "input: {input}");
        }
    }

    #[test]
    fn bool_lenient_rejects_garbage() {
        let err =
            serde_json::from_value::<FlagInput>(json!({"include_archived": "maybe"})).unwrap_err();
        assert!(err.to_string().contains("coerce string"), "got: {err}");
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct OptFlagInput {
        #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
        enabled: Option<bool>,
    }

    #[test]
    fn opt_bool_lenient_accepts_variants() {
        for (input, expected) in [
            (json!(true), Some(true)),
            (json!(false), Some(false)),
            (json!("true"), Some(true)),
            (json!("FALSE"), Some(false)),
            (json!("1"), Some(true)),
            (json!(0), Some(false)),
            (json!(null), None),
        ] {
            let v: OptFlagInput = serde_json::from_value(json!({"enabled": input})).unwrap();
            assert_eq!(v.enabled, expected, "input: {input}");
        }
    }

    #[test]
    fn opt_bool_lenient_accepts_missing() {
        let v: OptFlagInput = serde_json::from_value(json!({})).unwrap();
        assert_eq!(v.enabled, None);
    }

    #[test]
    fn opt_bool_lenient_rejects_garbage() {
        let err = serde_json::from_value::<OptFlagInput>(json!({"enabled": "maybe"})).unwrap_err();
        assert!(err.to_string().contains("coerce string"), "got: {err}");
    }
}
