// App commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    AddChannelRequest, AppRunBucket, AppRunEvent, AppRunListResponse, CreateAppChannelRow,
    CreateAppRequest, CreateAppRow, UpdateApp, UpdateAppChannel, UpdateAppRequest,
    UpdateChannelRequest,
};
use super::{APP_DANGEROUS, APP_MANAGE, APP_VIEW};
use crate::api::messages::{CreateMessageRequest, InputContentPart, InputMessage, MessageRole};
use crate::api::sessions::CreateSessionRequest;
use crate::auth::audit;
use crate::domains::common::*;
use crate::domains::messages::{CreateMessageContext, MessageService};
use crate::domains::sessions::SessionService;
use crate::errors::ResourceNotFoundError;
use crate::execution_metadata;
use crate::services::PrincipalService;
use crate::storage::models::AuditLogQuery;
use crate::storage::password::hash_password;
use chrono::{DateTime, Duration, Timelike, Utc};
use everruns_core::Policy;
use everruns_durable::{
    CreateScheduleRow, Pagination as DurablePagination, ScheduleExecutionFilter,
    ScheduleExecutionStatus, ScheduleTargetType, StoreError, UpdateField, UpdateSchedule,
    WorkflowEventStore,
};
use everruns_platform::app::{InvocationSessionMode, ScheduleChannelConfig, WebhookChannelConfig};
use everruns_platform::{
    A2aChannelConfig, AgUiChannelConfig, AgUiToolVisibility, ApiEndpointChannelConfig, App,
    AppChannel, AppEndpointAuthConfig, AppEndpointAuthMode, AppEndpointAuthProviderConfig,
    AppStatus, ChannelType, FcpChannelConfig, PublicChatChannelConfig, SlackChannelConfig,
};
use everruns_platform::{AgentAction, AuditEvent};
use everruns_provider::typed_id::{
    AgentId, AgentVersionId, AppChannelId, AppId, HarnessId, SessionId,
};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use utoipa::ToSchema;
use uuid::Uuid;

const APP_RUN_AUDIT_PAGE_SIZE: usize = 500;
const APP_RUN_AUDIT_MAX_PAGES: usize = 10;
const APP_RUN_AUDIT_MATCH_LIMIT: usize = 100;

// ============================================================================
// Validation helpers
// ============================================================================

async fn validate_harness(ctx: &Ctx, harness_id: HarnessId) -> Result<Uuid, CommandError> {
    let row = ctx
        .db
        .get_harness(ctx.org_id(), harness_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Harness").into()))?;
    if row.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted harnesses cannot be assigned"
        )));
    }
    Ok(row.id.uuid())
}

async fn validate_agent(ctx: &Ctx, agent_id: &AgentId) -> Result<Uuid, CommandError> {
    let row = ctx
        .db
        .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| classify_anyhow(ResourceNotFoundError::new("Agent").into()))?;
    if row.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted agents cannot be assigned"
        )));
    }
    Ok(row.id.uuid())
}

fn require_app_agent(agent_id: Option<Uuid>) -> Result<Uuid, CommandError> {
    agent_id.ok_or_else(|| {
        CommandError::bad_request(
            "Apps require an agent. Existing agent-less apps remain runnable, but must be assigned an agent before they can be updated.",
        )
    })
}

fn reject_new_schedule_channel(channel_type: &ChannelType) -> Result<(), CommandError> {
    if *channel_type == ChannelType::Schedule {
        return Err(CommandError::bad_request(
            "App schedule channels are deprecated. Create a schedule trigger on the app's agent instead.",
        ));
    }
    Ok(())
}

async fn validate_agent_identity(
    ctx: &Ctx,
    identity_id: everruns_provider::typed_id::AgentIdentityId,
) -> Result<Uuid, CommandError> {
    let identity = ctx
        .db
        .get_agent_identity(ctx.org_id(), identity_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent identity"))?;
    if identity.status != "active" {
        return Err(classify_anyhow(anyhow::anyhow!(
            "Archived or deleted agent identities cannot be assigned"
        )));
    }
    Ok(identity.id.uuid())
}

async fn validate_published_agent_version(
    ctx: &Ctx,
    agent_id: Option<Uuid>,
    version_id: AgentVersionId,
) -> Result<Uuid, CommandError> {
    let version = ctx
        .db
        .get_agent_version(ctx.org_id(), version_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent version"))?;
    if Some(version.agent_id.uuid()) != agent_id {
        return Err(CommandError::bad_request(
            "Agent version must belong to the selected agent",
        ));
    }
    if !version.is_published {
        return Err(CommandError::bad_request(
            "Agent version must be published before it can be pinned to an app",
        ));
    }
    Ok(version.id.uuid())
}

const SCHEDULE_CHANNEL_ACTIVITY: &str = "invoke_scheduled_app_channel";

static TEMPLATE_EXPR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*([a-zA-Z0-9_.-]+)\s*\}\}").expect("template regex is valid")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppInvocationSource {
    Schedule,
    Webhook,
    A2a,
    ApiEndpoint,
}

impl AppInvocationSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Webhook => "webhook",
            Self::A2a => "a2a",
            Self::ApiEndpoint => "api_endpoint",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppInvocationResult {
    pub session_id: SessionId,
    pub created_session: bool,
}

#[derive(Debug, Clone)]
pub struct WebhookInvocationRequest {
    pub app_id: String,
    pub channel_id: String,
    pub body: String,
    pub json_payload: Option<Value>,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct A2aInvocationRequest {
    pub app_id: String,
    pub channel_id: String,
    /// Verbatim A2A `params` object from the JSON-RPC envelope.
    pub params: Value,
    /// Concatenated text parts from `params.message.parts` (newline-joined).
    pub text: String,
    pub message_id: Option<String>,
    pub task_id: String,
    pub context_id: Option<String>,
    pub role: Option<String>,
}

fn durable_store(ctx: &Ctx) -> Result<&Arc<dyn WorkflowEventStore + Send + Sync>, CommandError> {
    ctx.workflow_store.as_ref().ok_or_else(|| {
        CommandError::bad_request("App schedule channels require durable execution to be enabled")
    })
}

/// Minimum seconds between consecutive schedule-channel triggers.
/// Configurable via `SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS`; default 300 (5 min).
fn schedule_channel_min_interval_seconds() -> i64 {
    const DEFAULT: i64 = 300;
    std::env::var("SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT)
}

/// Maximum number of enabled schedule channels per org.
/// Configurable via `SCHEDULE_CHANNEL_MAX_PER_ORG`; default 10.
fn schedule_channel_max_per_org() -> i64 {
    const DEFAULT: i64 = 10;
    std::env::var("SCHEDULE_CHANNEL_MAX_PER_ORG")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT)
}

pub(crate) fn normalize_cron_expression(cron_expression: &str) -> Result<String, CommandError> {
    let fields = cron_expression.split_whitespace().collect::<Vec<_>>();
    let normalized = match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        7 => fields.join(" "),
        _ => {
            return Err(CommandError::bad_request(
                "Cron expression must be either 5 fields (min hour day month weekday) or 7 fields (sec min hour day month weekday year)",
            ));
        }
    };
    cron::Schedule::from_str(&normalized).map_err(|e| {
        CommandError::bad_request(format!(
            "Invalid schedule cron expression '{cron_expression}': {e}"
        ))
    })?;
    Ok(normalized)
}

/// Returns the minimum interval in seconds between consecutive triggers.
///
/// Cron expressions without a year constraint are periodic over a full
/// leap-year-sized horizon, so scan that bounded window to catch non-uniform
/// bursts. If the horizon has too few occurrences, keep sampling upcoming
/// triggers without the horizon so future-dated bursts cannot bypass the limit.
pub(crate) fn cron_min_interval_seconds(
    schedule: &cron::Schedule,
    reject_below_seconds: i64,
) -> Option<i64> {
    const FALLBACK_OCCURRENCE_LIMIT: usize = 3;

    let start = Utc::now();
    let end = start + Duration::days(366);
    let mut previous: Option<DateTime<Utc>> = None;
    let mut min_interval: Option<i64> = None;
    let mut occurrences = 0;

    for next in schedule.after(&start).take_while(|next| *next <= end) {
        occurrences += 1;
        if let Some(previous) = previous {
            let interval = (next - previous).num_seconds();
            min_interval =
                Some(min_interval.map_or(interval, |current: i64| current.min(interval)));
            if interval < reject_below_seconds {
                return min_interval;
            }
        }
        previous = Some(next);
    }

    if occurrences >= 2 {
        return min_interval;
    }

    // Horizon had fewer than two occurrences (a future-dated or very sparse
    // schedule). Reset `previous` so the fallback measures intervals purely
    // among the sampled occurrences: otherwise the single horizon occurrence
    // would be compared against itself (the fallback restarts from `start`),
    // yielding a spurious 0-second interval that rejects a valid schedule.
    previous = None;
    for next in schedule.after(&start).take(FALLBACK_OCCURRENCE_LIMIT) {
        if let Some(previous) = previous {
            let interval = (next - previous).num_seconds();
            min_interval =
                Some(min_interval.map_or(interval, |current: i64| current.min(interval)));
            if interval < reject_below_seconds {
                break;
            }
        }
        previous = Some(next);
    }

    min_interval
}

/// Gate feature-flagged channel types. Public Chat is only available to
/// organizations with the `public_chat` feature flag enabled.
fn ensure_channel_type_enabled(ctx: &Ctx, channel_type: &ChannelType) -> Result<(), CommandError> {
    if *channel_type == ChannelType::PublicChat && !ctx.feature_flags.public_chat {
        return Err(CommandError::feature_not_enabled("public_chat"));
    }
    Ok(())
}

fn normalize_and_validate_channel_config(
    channel_type: ChannelType,
    mut channel_config: Value,
) -> Result<Value, CommandError> {
    match channel_type {
        ChannelType::AgUi
        | ChannelType::A2a
        | ChannelType::ApiEndpoint
        | ChannelType::PublicChat => {
            normalize_inline_endpoint_auth(&channel_type, &mut channel_config)?;
        }
        ChannelType::Fcp | ChannelType::Slack | ChannelType::Schedule | ChannelType::Webhook => {
            // FCP deliberately runs its own minimal auth stack (anonymous +
            // shared bearer token) so it never shares verifier code with
            // AG-UI/A2A. See `knowledge/integrations/fcp-channel.md`.
            if channel_config.get("auth").is_some() {
                return Err(CommandError::bad_request(
                    "App endpoint auth is only supported for AG-UI and A2A channels",
                ));
            }
        }
    }
    match channel_type {
        ChannelType::Slack => {
            let config: SlackChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid Slack channel config: {e}"))
                })?;
            if config.signing_secret.trim().is_empty() || config.bot_token.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Slack channel config requires non-empty signing_secret and bot_token",
                ));
            }
        }
        ChannelType::AgUi => {
            let config: AgUiChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid AG-UI channel config: {e}"))
                })?;
            // Reject obviously broken caps (e.g. > 1M req/min) so a typo can't
            // silently disable the per-app limit by overflowing reasonable
            // expectations. `0` is allowed and means "no per-app cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "AG-UI rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            if let Some(token) = config.token
                && token.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "AG-UI token must be non-empty when configured",
                ));
            }
            if config.generic_tool_text.chars().count() > 120 {
                return Err(CommandError::bad_request(
                    "AG-UI generic_tool_text must be at most 120 characters",
                ));
            }
            // Narrated also emits the configured generic_tool_text on public streams (the
            // raw narration is intentionally not forwarded), so non-empty text is required
            // for both Generic and Narrated.
            if matches!(
                config.tool_visibility,
                AgUiToolVisibility::Generic | AgUiToolVisibility::Narrated
            ) && config.generic_tool_text.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "AG-UI generic_tool_text cannot be empty when tool_visibility is generic or narrated",
                ));
            }
        }
        ChannelType::Schedule => {
            let config: ScheduleChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid schedule channel config: {e}"))
                })?;
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Schedule channel config requires a non-empty message",
                ));
            }
            let normalized = normalize_cron_expression(&config.cron_expression)?;
            // Enforce minimum cron interval.
            let schedule = cron::Schedule::from_str(&normalized).expect("already validated");
            let min_limit = schedule_channel_min_interval_seconds();
            if let Some(interval) = cron_min_interval_seconds(&schedule, min_limit)
                && interval < min_limit
            {
                return Err(CommandError::bad_request(format!(
                    "Schedule channel cron must fire no more than once every {min_limit} seconds (≥ {} min); expression fires every {interval} seconds",
                    min_limit / 60
                )));
            }
            if let Some(map) = channel_config.as_object_mut() {
                map.insert("cron_expression".to_string(), Value::String(normalized));
            }
        }
        ChannelType::Webhook => {
            let config: WebhookChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid webhook channel config: {e}"))
                })?;
            if config.token.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Webhook channel config requires a non-empty token",
                ));
            }
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "Webhook channel config requires a non-empty message",
                ));
            }
        }
        ChannelType::A2a => {
            let config: A2aChannelConfig =
                serde_json::from_value(channel_config.clone()).map_err(|e| {
                    CommandError::bad_request(format!("Invalid A2A channel config: {e}"))
                })?;
            if config.api_key_hash.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty api_key_hash",
                ));
            }
            if config.api_key_prefix.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty api_key_prefix",
                ));
            }
            if config.message.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "A2A channel config requires a non-empty message",
                ));
            }
            // Mirror the AG-UI cap so a typo in `rate_limit_per_minute` cannot
            // silently disable the per-channel limit by overflowing reasonable
            // expectations. `0` is allowed and means "no per-channel cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "A2A rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            // Whitespace-only `signing_secret` is almost certainly an
            // accidental write — reject it so the channel does not silently
            // start enforcing a signature with a useless key. `None` keeps
            // the current API-key-only behavior; an explicit non-empty
            // value opts the channel into HMAC replay protection
            // (TM-A2A-010). Cap the length so a single channel write
            // cannot bloat the encrypted column.
            if let Some(secret) = config.signing_secret.as_deref() {
                if secret.trim().is_empty() {
                    return Err(CommandError::bad_request(
                        "A2A signing_secret must be non-empty when configured",
                    ));
                }
                if secret.len() > 4096 {
                    return Err(CommandError::bad_request(
                        "A2A signing_secret must be at most 4096 bytes",
                    ));
                }
            }
        }
        ChannelType::ApiEndpoint => {
            let config: ApiEndpointChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                CommandError::bad_request(format!("Invalid api_endpoint channel config: {e}"))
            })?;
            if config.api_key_hash.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "api_endpoint channel config requires a non-empty api_key_hash",
                ));
            }
            if config.api_key_prefix.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "api_endpoint channel config requires a non-empty api_key_prefix",
                ));
            }
            // Mirror the A2A/AG-UI cap so a typo cannot silently disable the
            // per-channel limit by overflowing reasonable expectations. `0` is
            // allowed and means "no per-channel cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "api_endpoint rate_limit_per_minute must be at most 1,000,000",
                ));
            }
        }
        ChannelType::Fcp => {
            let config: FcpChannelConfig =
                serde_json::from_value(channel_config.clone()).map_err(|e| {
                    CommandError::bad_request(format!("Invalid FCP channel config: {e}"))
                })?;
            if let Some(token) = config.token.as_deref()
                && token.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "FCP token must be non-empty when configured",
                ));
            }
            if let Some(handshake) = config.handshake.as_deref()
                && handshake.len() > 8 * 1024
            {
                return Err(CommandError::bad_request(
                    "FCP handshake must be at most 8 KiB",
                ));
            }
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "FCP rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            if config.response_timeout_seconds == 0 || config.response_timeout_seconds > 600 {
                return Err(CommandError::bad_request(
                    "FCP response_timeout_seconds must be between 1 and 600",
                ));
            }
        }
        ChannelType::PublicChat => {
            let config: PublicChatChannelConfig = serde_json::from_value(channel_config.clone())
                .map_err(|e| {
                    CommandError::bad_request(format!("Invalid Public Chat channel config: {e}"))
                })?;
            // Mirror the AG-UI cap so a typo cannot silently disable the
            // per-app limit by overflowing reasonable expectations. `0` means
            // "no per-app cap".
            if let Some(limit) = config.rate_limit_per_minute
                && limit > 1_000_000
            {
                return Err(CommandError::bad_request(
                    "Public Chat rate_limit_per_minute must be at most 1,000,000",
                ));
            }
            if let Some(token) = config.token.as_deref()
                && token.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "Public Chat token must be non-empty when configured",
                ));
            }
            if config.generic_tool_text.chars().count() > 120 {
                return Err(CommandError::bad_request(
                    "Public Chat generic_tool_text must be at most 120 characters",
                ));
            }
            if matches!(
                config.tool_visibility,
                AgUiToolVisibility::Generic | AgUiToolVisibility::Narrated
            ) && config.generic_tool_text.trim().is_empty()
            {
                return Err(CommandError::bad_request(
                    "Public Chat generic_tool_text cannot be empty when tool_visibility is generic or narrated",
                ));
            }
            // An anonymous channel with no auth and no captcha is allowed (the
            // simplest "anyone with the link" case), but a captcha config must
            // be coherent: a non-empty site key, and a secret key on first
            // configuration (PATCH may omit it to preserve the stored value).
            if let Some(captcha) = config.captcha.as_ref() {
                if captcha.site_key.trim().is_empty() {
                    return Err(CommandError::bad_request(
                        "Public Chat captcha requires a non-empty site_key",
                    ));
                }
                if let Some(secret) = captcha.secret_key.as_deref()
                    && secret.trim().is_empty()
                {
                    return Err(CommandError::bad_request(
                        "Public Chat captcha secret_key must be non-empty when configured",
                    ));
                }
                // An enabled captcha with no stored secret would fail closed at
                // runtime (every anonymous request → 503). Require the secret
                // when enabled. On PATCH the existing secret is merged in before
                // this check, so editing other fields keeps working.
                let has_secret = captcha
                    .secret_key
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty());
                if captcha.enabled && !has_secret {
                    return Err(CommandError::bad_request(
                        "Public Chat captcha requires a secret_key when enabled",
                    ));
                }
            }
            // Branding sanity: cap the free-text fields so a single channel
            // write cannot bloat the encrypted config column.
            if let Some(name) = config.branding.display_name.as_deref()
                && name.chars().count() > 120
            {
                return Err(CommandError::bad_request(
                    "Public Chat branding display_name must be at most 120 characters",
                ));
            }
            if let Some(welcome) = config.branding.welcome_message.as_deref()
                && welcome.chars().count() > 2000
            {
                return Err(CommandError::bad_request(
                    "Public Chat branding welcome_message must be at most 2000 characters",
                ));
            }
        }
    }

    Ok(channel_config)
}

fn hash_app_endpoint_basic_password(password: &str) -> Result<String, CommandError> {
    hash_password(password).map_err(classify_anyhow)
}

fn normalize_inline_endpoint_auth(
    channel_type: &ChannelType,
    channel_config: &mut Value,
) -> Result<(), CommandError> {
    let Some(auth_value) = channel_config.get("auth") else {
        return Ok(());
    };
    if auth_value.is_null() {
        return Ok(());
    }
    let auth: AppEndpointAuthConfig = serde_json::from_value(auth_value.clone())
        .map_err(|e| CommandError::bad_request(format!("Invalid app endpoint auth config: {e}")))?;
    validate_endpoint_auth_config(channel_type, channel_config, &auth)?;

    if let Some(provider) = channel_config
        .get_mut("auth")
        .and_then(|auth| auth.get_mut("provider"))
        .and_then(Value::as_object_mut)
        && provider.get("type").and_then(Value::as_str) == Some("http_basic")
    {
        let password = provider
            .remove("password")
            .and_then(|value| value.as_str().map(str::to_owned));
        if let Some(password) = password {
            if password.trim().is_empty() {
                return Err(CommandError::bad_request(
                    "HTTP Basic password must be non-empty when configured",
                ));
            }
            provider.insert(
                "password_hash".to_string(),
                Value::String(hash_app_endpoint_basic_password(&password)?),
            );
        }
    }
    Ok(())
}

fn validate_endpoint_auth_config(
    channel_type: &ChannelType,
    channel_config: &Value,
    auth: &AppEndpointAuthConfig,
) -> Result<(), CommandError> {
    match auth.mode {
        AppEndpointAuthMode::Anonymous => Ok(()),
        AppEndpointAuthMode::SharedSecret => {
            if *channel_type != ChannelType::AgUi && *channel_type != ChannelType::PublicChat {
                return Err(CommandError::bad_request(
                    "Shared token auth is only supported for AG-UI and Public Chat channels",
                ));
            }
            if channel_config
                .get("token")
                .and_then(Value::as_str)
                .is_some_and(|token| !token.trim().is_empty())
            {
                Ok(())
            } else {
                Err(CommandError::bad_request(
                    "Shared token auth requires a non-empty token",
                ))
            }
        }
        AppEndpointAuthMode::ApiKey => {
            if *channel_type != ChannelType::A2a && *channel_type != ChannelType::ApiEndpoint {
                return Err(CommandError::bad_request(
                    "API key auth is only supported for A2A and api_endpoint channels",
                ));
            }
            let has_hash = channel_config
                .get("api_key_hash")
                .and_then(Value::as_str)
                .is_some_and(|hash| !hash.trim().is_empty());
            let has_prefix = channel_config
                .get("api_key_prefix")
                .and_then(Value::as_str)
                .is_some_and(|prefix| !prefix.trim().is_empty());
            if has_hash && has_prefix {
                Ok(())
            } else {
                Err(CommandError::bad_request(
                    "API key auth requires a configured API key",
                ))
            }
        }
        AppEndpointAuthMode::GoogleOidc => match auth.provider.as_ref() {
            Some(AppEndpointAuthProviderConfig::GoogleOidc { client_id, .. })
                if !client_id.trim().is_empty() =>
            {
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "Google auth requires provider.type=google_oidc and non-empty client_id",
            )),
        },
        AppEndpointAuthMode::Oidc => match auth.provider.as_ref() {
            Some(AppEndpointAuthProviderConfig::Oidc { issuer, jwks_url }) => {
                if issuer.trim().is_empty() {
                    return Err(CommandError::bad_request(
                        "OIDC auth requires a non-empty issuer",
                    ));
                }
                everruns_provider::url_validation::validate_safe_url(issuer)
                    .map_err(|e| CommandError::bad_request(format!("Invalid OIDC issuer: {e}")))?;
                if let Some(jwks_url) = jwks_url {
                    everruns_provider::url_validation::validate_safe_url(jwks_url).map_err(
                        |e| CommandError::bad_request(format!("Invalid OIDC JWKS URL: {e}")),
                    )?;
                }
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "OIDC auth requires provider.type=oidc",
            )),
        },
        AppEndpointAuthMode::OAuth2Introspection => match auth.provider.as_ref() {
            Some(AppEndpointAuthProviderConfig::OAuth2Introspection {
                introspection_url, ..
            }) => {
                everruns_provider::url_validation::validate_safe_url(introspection_url).map_err(
                    |e| CommandError::bad_request(format!("Invalid OAuth2 introspection URL: {e}")),
                )?;
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "OAuth2 introspection auth requires provider.type=oauth2_introspection",
            )),
        },
        AppEndpointAuthMode::HttpBasic => match auth.provider.as_ref() {
            Some(AppEndpointAuthProviderConfig::HttpBasic {
                username,
                password,
                password_hash,
            }) if !username.trim().is_empty()
                && (password.as_deref().is_some_and(|p| !p.trim().is_empty())
                    || password_hash
                        .as_deref()
                        .is_some_and(|hash| !hash.trim().is_empty())) =>
            {
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "HTTP Basic auth requires provider.type=http_basic, username, and password or password_hash",
            )),
        },
        AppEndpointAuthMode::Mtls => match auth.provider.as_ref() {
            Some(AppEndpointAuthProviderConfig::Mtls {
                header_name,
                allowed_values,
                proxy_secret_header,
                proxy_secret,
            }) if !header_name.trim().is_empty()
                && !allowed_values.is_empty()
                && proxy_secret_header
                    .as_deref()
                    .is_some_and(|h| !h.trim().is_empty())
                && proxy_secret
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty()) =>
            {
                Ok(())
            }
            _ => Err(CommandError::bad_request(
                "mTLS auth requires provider.type=mtls, header_name, allowed_values, proxy_secret_header, and proxy_secret",
            )),
        },
    }
}

pub(crate) fn calculate_schedule_next_trigger(
    cron_expression: &str,
) -> Result<Option<DateTime<Utc>>, CommandError> {
    let normalized = normalize_cron_expression(cron_expression)?;
    let schedule = cron::Schedule::from_str(&normalized).map_err(|e| {
        CommandError::bad_request(format!("Invalid cron expression '{cron_expression}': {e}"))
    })?;
    Ok(schedule.upcoming(Utc).next())
}

fn redact_channel_config(channel_type: &ChannelType, config: &mut Value) {
    redact_inline_endpoint_auth(config);
    let Some(map) = config.as_object_mut() else {
        return;
    };
    match channel_type {
        ChannelType::Slack => {
            if map.remove("signing_secret").is_some() {
                map.insert("signing_secret_configured".to_string(), Value::Bool(true));
            }
            if map.remove("bot_token").is_some() {
                map.insert("bot_token_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::AgUi => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::Webhook => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::A2a => {
            map.remove("api_key_hash");
            // signing_secret is write-only — the API redacts it on read
            // and only surfaces `signing_secret_configured: bool` so
            // operators can tell whether replay protection is on without
            // ever leaking the shared key. Mirrors the Slack /
            // webhook-token redaction pattern (TM-A2A-010). Only set the
            // flag when the stored value is a non-empty string; a
            // `null` / empty value means replay protection is **off**
            // and must not be advertised as configured.
            let removed = map.remove("signing_secret");
            let is_configured = removed
                .as_ref()
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty());
            if is_configured {
                map.insert("signing_secret_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::Fcp => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
        }
        ChannelType::ApiEndpoint => {
            // The api_key_hash is a secret-equivalent: anyone who can submit a
            // key whose SHA-256 matches it authenticates. Never surface it on
            // read; the non-secret api_key_prefix stays for display.
            map.remove("api_key_hash");
        }
        ChannelType::PublicChat => {
            if map.remove("token").is_some() {
                map.insert("token_configured".to_string(), Value::Bool(true));
            }
            // Turnstile secret key is write-only: surface only whether it is
            // configured so the site key (public) can still be returned for the
            // client widget without leaking the verification secret.
            if let Some(captcha) = map.get_mut("captcha").and_then(Value::as_object_mut) {
                let removed = captcha.remove("secret_key");
                let is_configured = removed
                    .as_ref()
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty());
                if is_configured {
                    captcha.insert("secret_key_configured".to_string(), Value::Bool(true));
                }
            }
        }
        ChannelType::Schedule => {}
    }
}

fn redact_inline_endpoint_auth(config: &mut Value) {
    let Some(provider) = config
        .get_mut("auth")
        .and_then(|auth| auth.get_mut("provider"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if provider.remove("password").is_some() || provider.remove("password_hash").is_some() {
        provider.insert("password_configured".to_string(), Value::Bool(true));
    }
    if provider.remove("client_secret").is_some() {
        provider.insert("client_secret_configured".to_string(), Value::Bool(true));
    }
    let has_proxy_secret = provider
        .get("proxy_secret")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    provider.remove("proxy_secret");
    if has_proxy_secret {
        provider.insert("proxy_secret_configured".to_string(), Value::Bool(true));
    }
}

fn redact_channel_for_response(mut channel: AppChannel) -> AppChannel {
    redact_channel_config(&channel.channel_type, &mut channel.channel_config);
    channel
}

fn redact_app_for_response(mut app: App) -> App {
    app.channels = app
        .channels
        .into_iter()
        .map(redact_channel_for_response)
        .collect();
    app
}

fn merge_preserved_secret_fields(
    channel_type: ChannelType,
    final_channel_config: &mut Value,
    existing_decrypted: &Value,
) {
    let (Some(out), Some(existing)) = (
        final_channel_config.as_object_mut(),
        existing_decrypted.as_object(),
    ) else {
        return;
    };

    match channel_type {
        ChannelType::Slack => {
            for key in ["signing_secret", "bot_token"] {
                let should_preserve = out
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_none_or(str::is_empty);
                if should_preserve && let Some(existing_value) = existing.get(key) {
                    out.insert(key.to_string(), existing_value.clone());
                }
            }
        }
        ChannelType::AgUi => {
            if !out.contains_key("token")
                && let Some(existing_value) = existing.get("token")
            {
                out.insert("token".to_string(), existing_value.clone());
            }
        }
        ChannelType::Webhook => {
            let should_preserve = out
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if should_preserve && let Some(existing_value) = existing.get("token") {
                out.insert("token".to_string(), existing_value.clone());
            }
        }
        ChannelType::A2a => {
            for key in ["api_key_hash", "api_key_prefix"] {
                if let Some(existing_value) = existing.get(key) {
                    out.insert(key.to_string(), existing_value.clone());
                }
            }
            // signing_secret is write-only on the wire — preserve the
            // existing value across PATCH so an operator editing the
            // session-mode / message / rate-limit field does not also
            // disable replay protection by omission (TM-A2A-010).
            if !out.contains_key("signing_secret")
                && let Some(existing_value) = existing.get("signing_secret")
            {
                out.insert("signing_secret".to_string(), existing_value.clone());
            }
        }
        ChannelType::Fcp => {
            let should_preserve = out
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if should_preserve && let Some(existing_value) = existing.get("token") {
                out.insert("token".to_string(), existing_value.clone());
            }
        }
        ChannelType::ApiEndpoint => {
            // api_key_hash is write-only on the wire (redacted on read), so a
            // PATCH that edits session_mode / rate_limit must preserve the
            // existing key rather than wipe it. Rotation goes through the
            // dedicated regenerate-key command.
            for key in ["api_key_hash", "api_key_prefix"] {
                if let Some(existing_value) = existing.get(key) {
                    out.insert(key.to_string(), existing_value.clone());
                }
            }
        }
        ChannelType::PublicChat => {
            let should_preserve = out
                .get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if should_preserve && let Some(existing_value) = existing.get("token") {
                out.insert("token".to_string(), existing_value.clone());
            }
            // Preserve the write-only Turnstile secret across a PATCH that
            // edits other captcha fields (e.g. toggling `enabled` or rotating
            // the site key) so the operator does not silently disable
            // verification by omitting the secret.
            if let (Some(out_captcha), Some(existing_captcha)) = (
                out.get_mut("captcha").and_then(Value::as_object_mut),
                existing.get("captcha").and_then(Value::as_object),
            ) {
                let should_preserve = out_captcha
                    .get("secret_key")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .is_none_or(str::is_empty);
                if should_preserve && let Some(existing_secret) = existing_captcha.get("secret_key")
                {
                    out_captcha.insert("secret_key".to_string(), existing_secret.clone());
                }
            }
        }
        ChannelType::Schedule => {}
    }
    merge_preserved_endpoint_auth_secrets(final_channel_config, existing_decrypted);
}

fn merge_preserved_endpoint_auth_secrets(
    final_channel_config: &mut Value,
    existing_decrypted: &Value,
) {
    let (Some(out_provider), Some(existing_provider)) = (
        final_channel_config
            .get_mut("auth")
            .and_then(|auth| auth.get_mut("provider"))
            .and_then(Value::as_object_mut),
        existing_decrypted
            .get("auth")
            .and_then(|auth| auth.get("provider"))
            .and_then(Value::as_object),
    ) else {
        return;
    };

    let same_type = out_provider.get("type").and_then(Value::as_str)
        == existing_provider.get("type").and_then(Value::as_str);
    if !same_type {
        return;
    }
    for key in ["password_hash", "client_secret", "proxy_secret"] {
        let should_preserve = out_provider
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .is_none_or(str::is_empty);
        if should_preserve && let Some(existing_value) = existing_provider.get(key) {
            out_provider.insert(key.to_string(), existing_value.clone());
        }
    }
}

fn schedule_binding_name(channel_id: AppChannelId) -> String {
    format!("app-channel-{channel_id}")
}

fn app_session_tags(app: &App, channel: &AppChannel) -> Vec<String> {
    vec![
        format!("app:{}", app.public_id),
        format!("app_channel:{}", channel.public_id),
        format!("app_channel_type:{}", channel.channel_type),
        "__internal:app_invocation".to_string(),
    ]
}

fn build_schedule_target_input(app: &App, channel: &AppChannel) -> Value {
    json!({
        "org_id": app.org_id,
        "app_id": app.public_id.to_string(),
        "channel_id": channel.public_id.to_string(),
    })
}

/// Metadata stamped onto the user message that an app channel injects into a
/// session. `_app_id` is the canonical "system metadata" key shared with the
/// AG-UI (`crates/server/src/api/ag_ui.rs`) and Slack
/// (`crates/server/src/api/slack_events.rs`) ingress paths; the rest of the
/// keys (`app_channel_id`, `app_channel_type`, `source`) are specific to the
/// schedule / webhook / A2A invocation channels documented in
/// `knowledge/integrations/app-invocation-channels.md` and are not emitted by AG-UI or Slack.
///
/// `_app_id` is the system-id consumed by `is_ag_ui_app_image` for AG-UI
/// image authorization and by the matching system-id assertions in the
/// channel ingress unit tests. The audit-log payload emitted by
/// `emit_app_invocation_audit_event` uses a bare `app_id` field on its own
/// `AuditEvent` struct for human-readable filtering; that is intentional and
/// lives separately from this metadata bag.
fn app_invocation_message_metadata(
    app: &App,
    channel: &AppChannel,
    source: AppInvocationSource,
) -> HashMap<String, Value> {
    [
        (
            "source".to_string(),
            Value::String(format!("app_{}", source.as_str())),
        ),
        (
            "_app_id".to_string(),
            Value::String(app.public_id.to_string()),
        ),
        (
            "app_channel_id".to_string(),
            Value::String(channel.public_id.to_string()),
        ),
        (
            "app_channel_type".to_string(),
            Value::String(channel.channel_type.to_string()),
        ),
    ]
    .into_iter()
    .collect()
}

fn emit_app_invocation_audit_event(
    db: Arc<crate::storage::StorageBackend>,
    app: &App,
    channel: &AppChannel,
    session_id: SessionId,
    source: AppInvocationSource,
    created_session: bool,
) {
    let mut event = AuditEvent::agent(AgentAction::AppInvocationStarted, app.org_id, None)
        .target("app_channel", channel.public_id.to_string())
        .detail("source", format!("app_{}", source.as_str()))
        .detail("app_id", app.public_id.to_string())
        .detail("app_channel_id", channel.public_id.to_string())
        .detail("app_channel_type", channel.channel_type.to_string())
        .detail("session_id", session_id.to_string())
        .detail("created_session", created_session);
    event = event.detail("app_owner_principal_id", app.owner_principal_id.to_string());
    if let Some(agent_identity_id) = app.agent_identity_id {
        event = event.detail("agent_identity_id", agent_identity_id.to_string());
    }
    audit::emit_event(db, event.build());
}

async fn set_channel_durable_schedule_id(
    ctx: &Ctx,
    channel_internal_id: Uuid,
    durable_schedule_id: UpdateField<Uuid>,
) -> Result<(), CommandError> {
    ctx.db
        .update_app_channel(
            channel_internal_id,
            UpdateAppChannel {
                durable_schedule_id,
                ..Default::default()
            },
        )
        .await
        .map_err(classify_anyhow)?;
    Ok(())
}

async fn delete_durable_schedule_if_present(
    store: &Arc<dyn WorkflowEventStore + Send + Sync>,
    durable_schedule_id: Option<Uuid>,
) -> Result<(), CommandError> {
    let Some(schedule_id) = durable_schedule_id else {
        return Ok(());
    };

    match store.delete_schedule(schedule_id).await {
        Ok(()) | Err(StoreError::ScheduleNotFound(_)) => Ok(()),
        Err(err) => Err(classify_anyhow(err.into())),
    }
}

async fn sync_schedule_binding_for_channel(
    ctx: &Ctx,
    app: &App,
    channel_row: &crate::storage::models::AppChannelRow,
) -> Result<(), CommandError> {
    let encryption = ctx.encryption.as_ref();
    let channel = q::channel_row_to_channel(encryption, channel_row.clone());

    if channel.channel_type != ChannelType::Schedule {
        if ctx.workflow_store.is_some() {
            delete_durable_schedule_if_present(
                durable_store(ctx)?,
                channel_row.durable_schedule_id,
            )
            .await?;
            if channel_row.durable_schedule_id.is_some() {
                set_channel_durable_schedule_id(ctx, channel_row.id, UpdateField::Clear).await?;
            }
        }
        return Ok(());
    }

    let store = durable_store(ctx)?;
    let config = channel
        .schedule_config()
        .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?;
    let cron_expression = normalize_cron_expression(&config.cron_expression)?;
    let enabled = app.status == AppStatus::Published && channel.enabled;
    let next_trigger_at = if enabled {
        UpdateField::from_option(calculate_schedule_next_trigger(&cron_expression)?)
    } else {
        UpdateField::Clear
    };

    let mut created_schedule_id = None;
    let schedule_id = match channel_row.durable_schedule_id {
        Some(schedule_id) => {
            let update = UpdateSchedule {
                name: Some(schedule_binding_name(channel.public_id)),
                description: UpdateField::Set(format!(
                    "App schedule channel {} for {}",
                    channel.public_id, app.name
                )),
                cron_expression: Some(cron_expression.clone()),
                timezone: Some(config.timezone.clone()),
                target_type: Some(ScheduleTargetType::Activity),
                target_name: Some(SCHEDULE_CHANNEL_ACTIVITY.to_string()),
                target_input: Some(build_schedule_target_input(app, &channel)),
                enabled: Some(enabled),
                max_concurrent: UpdateField::Set(1),
                catch_up_missed: Some(false),
                max_catch_up: UpdateField::Set(1),
                retry_policy: UpdateField::Clear,
                next_trigger_at,
            };

            match store.update_schedule(schedule_id, update).await {
                Ok(()) => schedule_id,
                Err(StoreError::ScheduleNotFound(_)) => {
                    let created = store
                        .create_schedule(CreateScheduleRow {
                            name: schedule_binding_name(channel.public_id),
                            description: Some(format!(
                                "App schedule channel {} for {}",
                                channel.public_id, app.name
                            )),
                            cron_expression: cron_expression.clone(),
                            timezone: config.timezone.clone(),
                            target_type: ScheduleTargetType::Activity,
                            target_name: SCHEDULE_CHANNEL_ACTIVITY.to_string(),
                            target_input: build_schedule_target_input(app, &channel),
                            enabled,
                            max_concurrent: Some(1),
                            catch_up_missed: false,
                            max_catch_up: Some(1),
                            retry_policy: None,
                            next_trigger_at: if enabled {
                                calculate_schedule_next_trigger(&cron_expression)?
                            } else {
                                None
                            },
                        })
                        .await
                        .map_err(|err| classify_anyhow(err.into()))?;
                    created_schedule_id = Some(created);
                    created
                }
                Err(err) => return Err(classify_anyhow(err.into())),
            }
        }
        None => {
            let created = store
                .create_schedule(CreateScheduleRow {
                    name: schedule_binding_name(channel.public_id),
                    description: Some(format!(
                        "App schedule channel {} for {}",
                        channel.public_id, app.name
                    )),
                    cron_expression: cron_expression.clone(),
                    timezone: config.timezone.clone(),
                    target_type: ScheduleTargetType::Activity,
                    target_name: SCHEDULE_CHANNEL_ACTIVITY.to_string(),
                    target_input: build_schedule_target_input(app, &channel),
                    enabled,
                    max_concurrent: Some(1),
                    catch_up_missed: false,
                    max_catch_up: Some(1),
                    retry_policy: None,
                    next_trigger_at: if enabled {
                        calculate_schedule_next_trigger(&cron_expression)?
                    } else {
                        None
                    },
                })
                .await
                .map_err(|err| classify_anyhow(err.into()))?;
            created_schedule_id = Some(created);
            created
        }
    };

    if channel_row.durable_schedule_id != Some(schedule_id)
        && let Err(err) =
            set_channel_durable_schedule_id(ctx, channel_row.id, UpdateField::Set(schedule_id))
                .await
    {
        if let Some(created_id) = created_schedule_id {
            let _ = store.delete_schedule(created_id).await;
        }
        return Err(err);
    }

    Ok(())
}

async fn sync_all_schedule_bindings(ctx: &Ctx, app: &App) -> Result<(), CommandError> {
    for channel_row in ctx
        .db
        .list_app_channels(app.internal_id)
        .await
        .map_err(classify_anyhow)?
    {
        sync_schedule_binding_for_channel(ctx, app, &channel_row).await?;
    }
    Ok(())
}

async fn require_app_channel_before_publish(
    ctx: &Ctx,
    app_internal_id: Uuid,
) -> Result<(), CommandError> {
    let has_channels = ctx
        .db
        .app_has_channels(app_internal_id)
        .await
        .map_err(classify_anyhow)?;
    if !has_channels {
        return Err(CommandError::bad_request(
            "App must have at least one channel before publishing",
        ));
    }
    Ok(())
}

async fn remove_schedule_binding_for_channel(
    ctx: &Ctx,
    channel_row: &crate::storage::models::AppChannelRow,
) -> Result<(), CommandError> {
    let Some(store) = ctx.workflow_store.as_ref() else {
        return Ok(());
    };
    delete_durable_schedule_if_present(store, channel_row.durable_schedule_id).await?;
    if channel_row.durable_schedule_id.is_some() {
        set_channel_durable_schedule_id(ctx, channel_row.id, UpdateField::Clear).await?;
    }
    Ok(())
}

fn template_lookup<'a>(context: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = context;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn template_value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub(crate) fn render_message_template(template: &str, context: &Value) -> String {
    TEMPLATE_EXPR_RE
        .replace_all(template, |captures: &regex::Captures<'_>| {
            let path = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            template_lookup(context, path)
                .map(template_value_to_string)
                .unwrap_or_default()
        })
        .into_owned()
}

fn shared_session_title(app: &App, source: AppInvocationSource) -> String {
    format!("{} {}", app.name, source.as_str())
}

fn invocation_session_title(app: &App, source: AppInvocationSource) -> String {
    format!(
        "{} {} {}",
        app.name,
        source.as_str(),
        Utc::now().to_rfc3339()
    )
}

async fn find_or_create_invocation_session(
    db: &Arc<crate::storage::StorageBackend>,
    session_service: &SessionService,
    app: &App,
    channel: &AppChannel,
    session_mode: InvocationSessionMode,
    source: AppInvocationSource,
) -> Result<(SessionId, bool), CommandError> {
    let shared_tags = app_session_tags(app, channel);
    if session_mode == InvocationSessionMode::SharedSession
        && let Some(existing) = db
            .find_app_session_by_tags_and_owner(
                app.org_id,
                app.internal_id,
                app.owner_principal_id,
                &shared_tags,
            )
            .await
            .map_err(classify_anyhow)?
    {
        return Ok((existing.id, false));
    }

    let mut tags = shared_tags;
    if session_mode == InvocationSessionMode::SessionPerInvocation {
        tags.push(format!("app_invocation:{}", Uuid::now_v7()));
    }

    let title = if session_mode == InvocationSessionMode::SharedSession {
        shared_session_title(app, source)
    } else {
        invocation_session_title(app, source)
    };

    let session = session_service
        .create_from_app(
            &everruns_core::Caller::internal(app.org_id),
            app.harness_id.uuid(),
            app.agent_id.map(|agent_id| agent_id.uuid()),
            app.agent_id,
            app.internal_id,
            // Pass the App's owner so the resulting session matches the
            // owner-keyed lookup in `find_app_session_by_tags_and_owner` —
            // shared-session reuse depends on this. See `create_from_app` doc.
            app.owner_principal_id,
            app.resolved_owner_user_id,
            // The invocation channel *is* the session's origin. `api_endpoint`
            // collapses into `webhook`: both are an inbound HTTP call into the app.
            match source {
                AppInvocationSource::Schedule => everruns_platform::SessionSource::Schedule,
                AppInvocationSource::Webhook | AppInvocationSource::ApiEndpoint => {
                    everruns_platform::SessionSource::Webhook
                }
                AppInvocationSource::A2a => everruns_platform::SessionSource::A2a,
            },
            CreateSessionRequest {
                source: None,
                workspace_id: None,
                harness_id: Some(app.harness_id),
                harness_name: None,
                agent_id: app.agent_id,
                agent_name: None,
                agent_identity_id: app.agent_identity_id,
                title: Some(title),
                goal: None,
                locale: None,
                tags,
                model_id: None,
                capabilities: vec![],
                tools: vec![],
                mcp_servers: Default::default(),
                system_prompt: None,
                initial_files: vec![],
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                parent_session_id: None,
                forked_from_session_id: None,
                budget_root_session_id: None,
                seed: everruns_core::SessionSeedMode::Fresh,
            },
        )
        .await
        .map_err(classify_anyhow)?;

    Ok((session.id, true))
}

async fn dispatch_invocation_message(
    message_service: &MessageService,
    app: &App,
    channel: &AppChannel,
    session_id: SessionId,
    source: AppInvocationSource,
    request_id: Option<String>,
    rendered_message: String,
) -> Result<(), CommandError> {
    let metadata = Some(app_invocation_message_metadata(app, channel, source));

    message_service
        .create(
            CreateMessageContext {
                org_id: app.org_id,
                user_id: None,
                harness_id: app.harness_id.uuid(),
                agent_id: app.agent_id.map(|agent_id| agent_id.uuid()),
                session_id: session_id.uuid(),
                event_metadata: Some(execution_metadata::app_message_metadata(
                    app.public_id,
                    app.owner_principal_id,
                    app.agent_identity_id,
                )),
                request_id,
            },
            CreateMessageRequest {
                message: InputMessage {
                    role: MessageRole::User,
                    content: vec![InputContentPart::text(rendered_message)],
                },
                addressed_participant_id: None,
                controls: None,
                metadata,
                tags: None,
                external_actor: None,
            },
        )
        .await
        .map_err(classify_anyhow)?;

    Ok(())
}

struct InvocationServices<'a> {
    db: &'a Arc<crate::storage::StorageBackend>,
    session_service: &'a SessionService,
    message_service: &'a MessageService,
}

struct InvocationRequest {
    app: App,
    channel: AppChannel,
    session_mode: InvocationSessionMode,
    source: AppInvocationSource,
    template_context: Value,
    request_id: Option<String>,
}

async fn invoke_app_channel_inner(
    services: InvocationServices<'_>,
    request: InvocationRequest,
) -> Result<AppInvocationResult, CommandError> {
    invoke_app_channel_inner_with_hook(services, request, |_session_id| async { Ok(()) }).await
}

/// Variant of [`invoke_app_channel_inner`] that runs a caller-supplied async
/// hook between session resolution and message dispatch. The hook gives
/// streaming callers a deterministic point to start an event subscription so
/// they cannot miss workflow events that the dispatched turn emits.
async fn invoke_app_channel_inner_with_hook<F, Fut>(
    services: InvocationServices<'_>,
    request: InvocationRequest,
    after_session_resolved: F,
) -> Result<AppInvocationResult, CommandError>
where
    F: FnOnce(SessionId) -> Fut,
    Fut: std::future::Future<Output = Result<(), CommandError>>,
{
    let InvocationRequest {
        app,
        channel,
        session_mode,
        source,
        template_context,
        request_id,
    } = request;

    if app.status != AppStatus::Published {
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    if !channel.enabled {
        return Err(CommandError::forbidden(
            "App channel is disabled".to_string(),
        ));
    }
    let message_template = match source {
        AppInvocationSource::Schedule => {
            channel
                .schedule_config()
                .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?
                .message
        }
        AppInvocationSource::Webhook => {
            channel
                .webhook_config()
                .ok_or_else(|| CommandError::bad_request("Invalid webhook channel configuration"))?
                .message
        }
        AppInvocationSource::A2a => {
            channel
                .a2a_config()
                .ok_or_else(|| CommandError::bad_request("Invalid A2A channel configuration"))?
                .message
        }
        // api_endpoint channels carry no config-side message template — the
        // caller supplies the message directly, so they use the dedicated
        // `invoke_api_app_channel` path instead of this template renderer.
        AppInvocationSource::ApiEndpoint => {
            return Err(CommandError::bad_request(
                "api_endpoint channels do not use message templates",
            ));
        }
    };
    let rendered_message = render_message_template(&message_template, &template_context);
    if rendered_message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "Rendered invocation message is empty",
        ));
    }

    let (session_id, created_session) = find_or_create_invocation_session(
        services.db,
        services.session_service,
        &app,
        &channel,
        session_mode,
        source,
    )
    .await?;

    // Subscribe-before-dispatch hook: streaming callers register here so the
    // workflow events emitted by `dispatch_invocation_message` cannot race
    // ahead of the SSE subscription.
    after_session_resolved(session_id).await?;

    dispatch_invocation_message(
        services.message_service,
        &app,
        &channel,
        session_id,
        source,
        request_id,
        rendered_message,
    )
    .await?;

    emit_app_invocation_audit_event(
        Arc::clone(services.db),
        &app,
        &channel,
        session_id,
        source,
        created_session,
    );

    Ok(AppInvocationResult {
        session_id,
        created_session,
    })
}

pub async fn invoke_scheduled_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    org_id: i64,
    app_id: &str,
    channel_id: &str,
) -> Result<AppInvocationResult, CommandError> {
    let app = q::get_by_public_id(db, encryption, org_id, app_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("App"))?;
    let channel_public_id: AppChannelId = channel_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app
        .channel_by_id(&channel_public_id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    let config = channel
        .schedule_config()
        .ok_or_else(|| CommandError::bad_request("Invalid schedule channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": app.public_id.to_string(),
            "name": app.name.clone(),
        },
        "channel": {
            "id": channel.public_id.to_string(),
            "type": channel.channel_type.to_string(),
        },
        "invocation": {
            "source": "schedule",
            "triggered_at": Utc::now().to_rfc3339(),
        },
    });

    invoke_app_channel_inner(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            app,
            channel,
            session_mode: config.session_mode,
            source: AppInvocationSource::Schedule,
            template_context,
            request_id: None,
        },
    )
    .await
}

pub async fn invoke_a2a_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: A2aInvocationRequest,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    invoke_a2a_app_channel_with_hook(
        db,
        encryption,
        session_service,
        message_service,
        req,
        request_id,
        |_session_id| async { Ok(()) },
    )
    .await
}

/// Variant of [`invoke_a2a_app_channel`] that runs a caller-supplied async
/// hook between session resolution and message dispatch. Streaming callers
/// use the hook to subscribe to session events at the safe point — before
/// the durable workflow that the dispatched message triggers can emit any
/// translatable events.
pub async fn invoke_a2a_app_channel_with_hook<F, Fut>(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: A2aInvocationRequest,
    request_id: Option<String>,
    after_session_resolved: F,
) -> Result<AppInvocationResult, CommandError>
where
    F: FnOnce(SessionId) -> Fut,
    Fut: std::future::Future<Output = Result<(), CommandError>>,
{
    let app = q::get_by_public_id_unscoped(db, encryption, &req.app_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("App"))?;
    let channel_public_id: AppChannelId = req
        .channel_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app
        .channel_by_id(&channel_public_id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    let config = channel
        .a2a_config()
        .ok_or_else(|| CommandError::bad_request("Invalid A2A channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": app.public_id.to_string(),
            "name": app.name.clone(),
        },
        "channel": {
            "id": channel.public_id.to_string(),
            "type": channel.channel_type.to_string(),
        },
        "invocation": {
            "source": "a2a",
            "triggered_at": Utc::now().to_rfc3339(),
        },
        "payload": req.params.clone(),
        "a2a": {
            "text": req.text,
            "message_id": req.message_id,
            "task_id": req.task_id,
            "context_id": req.context_id,
            "role": req.role,
        },
    });

    invoke_app_channel_inner_with_hook(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            app,
            channel,
            session_mode: config.session_mode,
            source: AppInvocationSource::A2a,
            template_context,
            request_id,
        },
        after_session_resolved,
    )
    .await
}

/// Request to start an api_endpoint execution-key invocation.
#[derive(Debug, Clone)]
pub struct ApiInvocationRequest {
    pub app_id: String,
    pub channel_id: String,
    /// Caller-supplied message dispatched into the app session.
    pub message: String,
}

/// Resolve the published app + enabled api_endpoint channel for an
/// execution-key request. Shared by the create-session and post-message paths
/// **and** by the HTTP auth layer (`api::app_api::authenticate_request`) so the
/// published / enabled / channel-type gate lives in exactly one place and
/// cannot drift between the two.
pub async fn resolve_api_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    app_id: &str,
    channel_id: &str,
) -> Result<(App, AppChannel), CommandError> {
    let app = q::get_by_public_id_unscoped(db, encryption, app_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("App"))?;
    if app.status != AppStatus::Published {
        return Err(CommandError::forbidden("App is not published".to_string()));
    }
    let channel_public_id: AppChannelId = channel_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app
        .channel_by_id(&channel_public_id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    if channel.channel_type != ChannelType::ApiEndpoint {
        return Err(CommandError::not_found("Channel"));
    }
    if !channel.enabled {
        return Err(CommandError::forbidden(
            "App channel is disabled".to_string(),
        ));
    }
    Ok((app, channel))
}

/// Whether a session's routing tags bind it to the given app + channel.
/// Confinement check for api_endpoint execution keys (mirrors the A2A
/// `session_belongs_to_a2a_channel` guard). THREAT[TM-APIKEY-002].
pub fn session_has_app_channel_tags(
    tags: &[String],
    app_public_id: &str,
    channel_public_id: &str,
) -> bool {
    let app_tag = format!("app:{app_public_id}");
    let channel_tag = format!("app_channel:{channel_public_id}");
    tags.iter().any(|t| t == &app_tag) && tags.iter().any(|t| t == &channel_tag)
}

/// Create (or resolve, for shared-session mode) the app-owned session for an
/// api_endpoint channel and dispatch the caller-supplied message, triggering a
/// turn. Mirrors `invoke_a2a_app_channel`, but the message is supplied by the
/// caller rather than rendered from a config-side template.
pub async fn invoke_api_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: ApiInvocationRequest,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    if req.message.trim().is_empty() {
        return Err(CommandError::bad_request("message must not be empty"));
    }
    let (app, channel) =
        resolve_api_app_channel(db, encryption, &req.app_id, &req.channel_id).await?;
    let config = channel
        .api_endpoint_config()
        .ok_or_else(|| CommandError::bad_request("Invalid api_endpoint channel configuration"))?;

    let (session_id, created_session) = find_or_create_invocation_session(
        db,
        session_service,
        &app,
        &channel,
        config.session_mode,
        AppInvocationSource::ApiEndpoint,
    )
    .await?;

    dispatch_invocation_message(
        message_service,
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        request_id,
        req.message,
    )
    .await?;

    emit_app_invocation_audit_event(
        Arc::clone(db),
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        created_session,
    );

    Ok(AppInvocationResult {
        session_id,
        created_session,
    })
}

/// Dispatch a follow-up message into an existing session that belongs to the
/// api_endpoint channel. The session must carry the channel's routing tags
/// (confinement) or the call fails with not-found, so one app's key cannot
/// drive another app's sessions. THREAT[TM-APIKEY-002].
#[allow(clippy::too_many_arguments)]
pub async fn post_api_app_channel_message(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    message_service: &MessageService,
    app_id: &str,
    channel_id: &str,
    session_id: SessionId,
    message: String,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    if message.trim().is_empty() {
        return Err(CommandError::bad_request("message must not be empty"));
    }
    let (app, channel) = resolve_api_app_channel(db, encryption, app_id, channel_id).await?;

    let session = db
        .get_session(app.org_id, session_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    if !session_has_app_channel_tags(
        &session.tags,
        &app.public_id.to_string(),
        &channel.public_id.to_string(),
    ) {
        return Err(CommandError::not_found("Session"));
    }

    dispatch_invocation_message(
        message_service,
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        request_id,
        message,
    )
    .await?;

    emit_app_invocation_audit_event(
        Arc::clone(db),
        &app,
        &channel,
        session_id,
        AppInvocationSource::ApiEndpoint,
        false,
    );

    Ok(AppInvocationResult {
        session_id,
        created_session: false,
    })
}

pub async fn invoke_webhook_app_channel(
    db: &Arc<crate::storage::StorageBackend>,
    encryption: Option<&Arc<crate::storage::encryption::EncryptionService>>,
    session_service: &SessionService,
    message_service: &MessageService,
    req: WebhookInvocationRequest,
    request_id: Option<String>,
) -> Result<AppInvocationResult, CommandError> {
    let app = q::get_by_public_id_unscoped(db, encryption, &req.app_id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("App"))?;
    let channel_public_id: AppChannelId = req
        .channel_id
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid channel ID: {e}")))?;
    let channel = app
        .channel_by_id(&channel_public_id)
        .cloned()
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    let config = channel
        .webhook_config()
        .ok_or_else(|| CommandError::bad_request("Invalid webhook channel configuration"))?;
    let template_context = json!({
        "app": {
            "id": app.public_id.to_string(),
            "name": app.name.clone(),
        },
        "channel": {
            "id": channel.public_id.to_string(),
            "type": channel.channel_type.to_string(),
        },
        "invocation": {
            "source": "webhook",
            "triggered_at": Utc::now().to_rfc3339(),
        },
        "payload": req
            .json_payload
            .clone()
            .unwrap_or_else(|| Value::String(req.body.clone())),
        "webhook": {
            "body": req.body,
            "json": req.json_payload,
            "headers": req.headers,
        },
    });

    invoke_app_channel_inner(
        InvocationServices {
            db,
            session_service,
            message_service,
        },
        InvocationRequest {
            app,
            channel,
            session_mode: config.session_mode,
            source: AppInvocationSource::Webhook,
            template_context,
            request_id,
        },
    )
    .await
}

// ============================================================================
// CreateApp
// ============================================================================

/// Create a new app with a harness, agent, and optional channel.
#[derive(Debug, Deserialize)]
pub struct CreateApp(pub CreateAppRequest);

impl CommandSchema for CreateApp {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateAppRequest>()
    }
}

impl Command for CreateApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_app",
            category: "apps",
            description: "Create a new app with a harness, agent, and optional channel.",
            method: "POST",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let CreateAppRequest {
            name,
            description,
            harness_id,
            agent_id,
            agent_version_policy,
            agent_version_id,
            agent_identity_id,
            channel_type,
            channel_config,
        } = self.0;
        let encryption = ctx.encryption.as_ref();

        if channel_type.is_none() && channel_config.is_some() {
            return Err(CommandError::bad_request(
                "channel_config requires channel_type",
            ));
        }

        let mut channel_config = channel_config.unwrap_or_default();

        if let Some(channel_type) = channel_type.clone() {
            reject_new_schedule_channel(&channel_type)?;
            ensure_channel_type_enabled(ctx, &channel_type)?;
            if channel_type == ChannelType::Schedule {
                let _ = durable_store(ctx)?;
                // Soft cap: count then create (TOCTOU window is intentional — this is a
                // noisy-neighbor limit, not a hard security boundary; strict serialization
                // is not worth the complexity).
                let count = ctx
                    .db
                    .count_enabled_schedule_channels_for_org(ctx.org_id())
                    .await
                    .map_err(classify_anyhow)?;
                let max = schedule_channel_max_per_org();
                if count >= max {
                    return Err(CommandError::bad_request(format!(
                        "Organization may have at most {max} enabled schedule channel(s); currently has {count}"
                    )));
                }
            }
            channel_config = normalize_and_validate_channel_config(channel_type, channel_config)?;
        }

        // Validate references
        let harness_uuid = validate_harness(ctx, harness_id).await?;
        let agent_uuid = match agent_id {
            Some(agent_id) => Some(validate_agent(ctx, &agent_id).await?),
            None => None,
        };
        let agent_uuid = Some(require_app_agent(agent_uuid)?);
        let agent_version_uuid = if let Some(version_id) = agent_version_id {
            Some(validate_published_agent_version(ctx, agent_uuid, version_id).await?)
        } else {
            None
        };
        let agent_identity_uuid = if let Some(identity_id) = agent_identity_id {
            Some(validate_agent_identity(ctx, identity_id).await?)
        } else {
            None
        };
        let owner_principal = PrincipalService::new(ctx.db.clone())
            .default_owner_principal(&ctx.caller, agent_identity_id)
            .await
            .map_err(classify_anyhow)?;

        // Prepare channel config
        let (stored_plaintext, channel_config_encrypted) =
            q::prepare_channel_config(encryption, &channel_config).map_err(classify_anyhow)?;

        // Persist app
        let internal_uuid = Uuid::now_v7();
        let public_id = AppId::from_uuid(internal_uuid);
        let input = CreateAppRow {
            public_id: public_id.to_string(),
            name,
            description,
            harness_id: harness_uuid,
            agent_id: agent_uuid,
            agent_version_policy: agent_version_policy.to_string(),
            agent_version_id: agent_version_uuid,
            agent_identity_id: agent_identity_uuid,
            owner_principal_id: owner_principal.id,
            resolved_owner_user_id: owner_principal.resolved_user_id,
            channel_type: channel_type.as_ref().map(ToString::to_string),
            channel_config: stored_plaintext.clone(),
            channel_config_encrypted: channel_config_encrypted.clone(),
        };
        let row = ctx
            .db
            .create_app(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;

        let mut created_channel = None;
        if let Some(channel_type) = channel_type {
            let channel_uuid = Uuid::now_v7();
            let channel_public_id = AppChannelId::from_uuid(channel_uuid);
            let channel_input = CreateAppChannelRow {
                public_id: channel_public_id.to_string(),
                channel_type: channel_type.to_string(),
                channel_config: stored_plaintext,
                channel_config_encrypted,
                durable_schedule_id: None,
                enabled: true,
            };
            let channel_row = ctx
                .db
                .create_app_channel(row.id, channel_input)
                .await
                .map_err(classify_anyhow)?;
            created_channel = Some(channel_row);
        }

        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        if let Some(channel_row) = created_channel.as_ref() {
            sync_schedule_binding_for_channel(ctx, &app, channel_row).await?;
        }

        Ok(redact_app_for_response(app))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateApp>() }

// ============================================================================
// ListApps
// ============================================================================

/// List apps. Supports search and include_archived.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListApps {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

impl Command for ListApps {
    type Output = Vec<App>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_apps",
            category: "apps",
            description: "List all apps. Use search for name search, include_archived=true to include archived.",
            method: "GET",
            path: "/v1/apps",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<App>, CommandError> {
        let encryption = ctx.encryption.as_ref();
        let rows = ctx
            .db
            .list_apps(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        Ok(q::load_apps_list(&ctx.db, encryption, rows, ctx.org_id())
            .await
            .map_err(classify_anyhow)?
            .into_iter()
            .map(redact_app_for_response)
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListApps>() }

// ============================================================================
// GetApp
// ============================================================================

/// Get a single app by ID.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetApp {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for GetApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_app",
            category: "apps",
            description: "Get a single app by ID.",
            method: "GET",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        q::get_by_public_id(&ctx.db, encryption, ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .map(redact_app_for_response)
            .ok_or_else(|| CommandError::not_found("App"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetApp>() }

// ============================================================================
// ListAppChannels
// ============================================================================

/// List channels attached to an app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAppChannels {
    /// App's prefixed public identifier.
    pub app_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_type: Option<ChannelType>,
}

impl Command for ListAppChannels {
    type Output = Vec<AppChannel>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_app_channels",
            category: "apps",
            description: "List channels attached to an app. Secret fields are redacted.",
            method: "GET",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("app_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AppChannel>, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        let app = q::get_by_public_id(&ctx.db, encryption, ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        Ok(app
            .channels
            .into_iter()
            .filter(|channel| {
                self.channel_type
                    .as_ref()
                    .is_none_or(|channel_type| &channel.channel_type == channel_type)
            })
            .map(redact_channel_for_response)
            .collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAppChannels>() }

// ============================================================================
// ListAppRuns
// ============================================================================

/// List recent app channel invocations for the app detail Live activity UI.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListAppRuns {
    pub app_id: String,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default, rename = "groupBy")]
    pub group_by: Option<String>,
}

impl Command for ListAppRuns {
    type Output = AppRunListResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_app_runs",
            category: "apps",
            description: "List recent app channel invocation runs.",
            method: "GET",
            path: "/v1/apps/{id}/runs",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("app_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppRunListResponse, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;
        let group_by_hour = match self.group_by.as_deref() {
            None => false,
            Some("hour") => true,
            Some(_) => return Err(CommandError::bad_request("groupBy must be 'hour'")),
        };
        let cutoff = Utc::now() - parse_run_history_window(self.window.as_deref())?;

        let encryption = ctx.encryption.as_ref();
        let app = q::get_by_public_id(&ctx.db, encryption, ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        let mut channel_by_id: HashMap<String, ChannelType> = app
            .channels
            .iter()
            .map(|channel| (channel.public_id.to_string(), channel.channel_type.clone()))
            .collect();
        let mut schedule_channels = HashMap::new();
        for channel_row in ctx
            .db
            .list_app_channels(app.internal_id)
            .await
            .map_err(classify_anyhow)?
        {
            if channel_row.channel_type == ChannelType::Schedule.to_string()
                && let Some(schedule_id) = channel_row.durable_schedule_id
                && let Some(channel_type) = ChannelType::from_str_opt(&channel_row.channel_type)
            {
                channel_by_id.insert(channel_row.public_id.clone(), channel_type.clone());
                schedule_channels.insert(schedule_id, (channel_row.public_id, channel_type));
            }
        }

        let mut runs = Vec::new();
        let mut loaded_schedule_runs = false;
        if !schedule_channels.is_empty()
            && let Some(store) = &ctx.workflow_store
        {
            for (schedule_id, (channel_id, channel_type)) in &schedule_channels {
                let executions = store
                    .list_schedule_executions(
                        ScheduleExecutionFilter {
                            schedule_id: Some(*schedule_id),
                            status: None,
                        },
                        DurablePagination {
                            offset: 0,
                            limit: 200,
                        },
                    )
                    .await
                    .map_err(|err| classify_anyhow(err.into()))?;
                runs.extend(
                    executions
                        .into_iter()
                        .filter(|execution| execution.scheduled_at >= cutoff)
                        .map(|execution| AppRunEvent {
                            id: execution.id.to_string(),
                            app_id: app.public_id.to_string(),
                            channel_id: channel_id.clone(),
                            channel_type: channel_type.clone(),
                            channel_name: None,
                            status: schedule_execution_status(&execution.status).to_string(),
                            created_at: execution.scheduled_at,
                            completed_at: execution.completed_at,
                        }),
                );
            }
            loaded_schedule_runs = true;
        }

        let app_id_string = app.public_id.to_string();
        let mut audit_before = None;
        let mut audit_matches = 0usize;
        for _ in 0..APP_RUN_AUDIT_MAX_PAGES {
            let audit_rows = ctx
                .db
                .list_audit_logs(AuditLogQuery {
                    org_id: ctx.org_id(),
                    limit: APP_RUN_AUDIT_PAGE_SIZE as i64,
                    before: audit_before,
                    event_type_prefix: None,
                    actor_id: None,
                    domain: Some("agent"),
                    action: Some("agent.app_invocation.started"),
                })
                .await
                .map_err(classify_anyhow)?;
            if audit_rows.is_empty() {
                break;
            }

            let page_len = audit_rows.len();
            let next_before = audit_rows.last().map(|row| row.created_at);
            let reached_cutoff = audit_rows.last().is_some_and(|row| row.created_at < cutoff);
            for row in audit_rows {
                if row.created_at < cutoff
                    || row.metadata.get("app_id").and_then(Value::as_str)
                        != Some(app_id_string.as_str())
                {
                    continue;
                }
                let Some(channel_id) = row
                    .metadata
                    .get("app_channel_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                let Some(channel_type) = row
                    .metadata
                    .get("app_channel_type")
                    .and_then(Value::as_str)
                    .and_then(ChannelType::from_str_opt)
                    .or_else(|| channel_by_id.get(&channel_id).cloned())
                else {
                    continue;
                };
                if channel_type == ChannelType::Schedule && loaded_schedule_runs {
                    continue;
                }
                runs.push(AppRunEvent {
                    id: row.id.to_string(),
                    app_id: app_id_string.clone(),
                    channel_id,
                    channel_type,
                    channel_name: None,
                    status: "running".to_string(),
                    created_at: row.created_at,
                    completed_at: None,
                });
                audit_matches += 1;
                if audit_matches >= APP_RUN_AUDIT_MATCH_LIMIT {
                    break;
                }
            }
            if page_len < APP_RUN_AUDIT_PAGE_SIZE
                || reached_cutoff
                || audit_matches >= APP_RUN_AUDIT_MATCH_LIMIT
            {
                break;
            }
            audit_before = next_before;
        }

        runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
        runs.truncate(100);
        let buckets = group_by_hour.then(|| bucket_app_runs_by_hour(&runs));
        Ok(AppRunListResponse {
            data: runs,
            buckets,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListAppRuns>() }

fn parse_run_history_window(window: Option<&str>) -> Result<Duration, CommandError> {
    const MAX_WINDOW_DAYS: i64 = 30;
    const MAX_WINDOW_MINUTES: i64 = MAX_WINDOW_DAYS * 24 * 60;

    let Some(window) = window else {
        return Ok(Duration::hours(24));
    };
    if window.len() < 2 || !window.is_ascii() {
        return Err(CommandError::bad_request(
            "window must use m, h, or d units, such as 24h",
        ));
    }
    let (amount, unit) = window.split_at(window.len().saturating_sub(1));
    let value: i64 = amount
        .parse()
        .map_err(|_| CommandError::bad_request("window must use a positive integer amount"))?;
    if value <= 0 {
        return Err(CommandError::bad_request("window must be positive"));
    }
    let max_for_unit = match unit {
        "m" => MAX_WINDOW_MINUTES,
        "h" => MAX_WINDOW_DAYS * 24,
        "d" => MAX_WINDOW_DAYS,
        _ => {
            return Err(CommandError::bad_request(
                "window must use m, h, or d units, such as 24h",
            ));
        }
    };
    if value > max_for_unit {
        return Err(CommandError::bad_request(
            "window exceeds the 30-day maximum",
        ));
    }
    let duration = match unit {
        "m" => Duration::try_minutes(value),
        "h" => Duration::try_hours(value),
        "d" => Duration::try_days(value),
        _ => unreachable!("unit already validated"),
    }
    .ok_or_else(|| CommandError::bad_request("window is out of range"))?;
    Ok(duration)
}

fn schedule_execution_status(status: &ScheduleExecutionStatus) -> &'static str {
    match status {
        ScheduleExecutionStatus::Pending => "pending",
        ScheduleExecutionStatus::Running => "running",
        ScheduleExecutionStatus::Completed => "completed",
        ScheduleExecutionStatus::Failed => "failed",
        ScheduleExecutionStatus::Skipped => "skipped",
    }
}

fn bucket_app_runs_by_hour(runs: &[AppRunEvent]) -> Vec<AppRunBucket> {
    let mut buckets: BTreeMap<DateTime<Utc>, AppRunBucket> = BTreeMap::new();
    for run in runs {
        let hour = run
            .created_at
            .with_minute(0)
            .and_then(|ts| ts.with_second(0))
            .and_then(|ts| ts.with_nanosecond(0))
            .unwrap_or(run.created_at);
        let bucket = buckets.entry(hour).or_insert_with(|| AppRunBucket {
            hour,
            ..Default::default()
        });
        match run.status.as_str() {
            "completed" => bucket.ok += 1,
            "failed" | "skipped" => bucket.err += 1,
            _ => {
                let running = bucket.running.unwrap_or(0) + 1;
                bucket.running = Some(running);
            }
        }
    }
    buckets.into_values().collect()
}

// ============================================================================
// UpdateApp
// ============================================================================

/// Update an app. Only provided fields are changed.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAppCmd {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateAppRequest,
}

impl Command for UpdateAppCmd {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_app",
            category: "apps",
            description: "Update an app. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let req = self.req;
        if matches!(req.status, Some(AppStatus::Deleted)) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(existing.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }
        if matches!(req.status, Some(AppStatus::Published | AppStatus::Draft)) {
            return Err(CommandError::bad_request(
                "Use publish/unpublish endpoints to change app publish status",
            ));
        }

        // Validate optional references
        let harness_id = if let Some(hid) = req.harness_id {
            Some(validate_harness(ctx, hid).await?)
        } else {
            None
        };

        let agent_id = if let Some(aid) = req.agent_id {
            Some(validate_agent(ctx, &aid).await?)
        } else {
            None
        };
        let effective_agent_id = agent_id.or(existing.agent_id);
        require_app_agent(effective_agent_id)?;
        let agent_version_id = match req.agent_version_id {
            UpdateField::Set(version_id) => UpdateField::Set(
                validate_published_agent_version(ctx, effective_agent_id, version_id).await?,
            ),
            UpdateField::Clear => UpdateField::Clear,
            UpdateField::Unchanged => UpdateField::Unchanged,
        };

        let agent_identity_id = match req.agent_identity_id {
            UpdateField::Set(identity_id) => {
                let uuid = validate_agent_identity(ctx, identity_id).await?;
                UpdateField::Set(uuid)
            }
            UpdateField::Clear => UpdateField::Clear,
            UpdateField::Unchanged => UpdateField::Unchanged,
        };
        let (owner_principal_id, resolved_owner_user_id) = match agent_identity_id {
            UpdateField::Set(identity_uuid) => {
                let owner = PrincipalService::new(ctx.db.clone())
                    .owner_for_entity(
                        ctx.org_id(),
                        existing.owner_principal_id,
                        existing.resolved_owner_user_id,
                        Some(everruns_provider::typed_id::AgentIdentityId::from_uuid(
                            identity_uuid,
                        )),
                    )
                    .await
                    .map_err(classify_anyhow)?;
                (
                    Some(owner.id),
                    UpdateField::from_option(owner.resolved_user_id),
                )
            }
            UpdateField::Clear => {
                let owner = PrincipalService::new(ctx.db.clone())
                    .owner_for_entity(
                        ctx.org_id(),
                        existing.owner_principal_id,
                        existing.resolved_owner_user_id,
                        None,
                    )
                    .await
                    .map_err(classify_anyhow)?;
                (
                    Some(owner.id),
                    UpdateField::from_option(owner.resolved_user_id),
                )
            }
            UpdateField::Unchanged => (None, UpdateField::Unchanged),
        };

        let encryption = ctx.encryption.as_ref();
        let mut channel_config = None;
        let mut channel_config_encrypted = None;
        if existing.channel_config_encrypted.is_none() && encryption.is_some() {
            let (stored, encrypted) =
                q::prepare_channel_config(encryption, &existing.channel_config)
                    .map_err(classify_anyhow)?;
            channel_config = Some(stored);
            channel_config_encrypted = encrypted;
        }

        let input = UpdateApp {
            name: req.name,
            description: req.description,
            harness_id,
            agent_id,
            agent_version_policy: req.agent_version_policy.map(|policy| policy.to_string()),
            agent_version_id,
            agent_identity_id,
            owner_principal_id,
            resolved_owner_user_id,
            channel_type: None,
            channel_config,
            channel_config_encrypted,
            status: req.status.clone().map(|s| s.to_string()),
            published_at: UpdateField::Unchanged,
        };

        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if encryption.is_some() {
            let channel_rows = ctx
                .db
                .list_app_channels(existing.id)
                .await
                .map_err(classify_anyhow)?;
            for channel_row in channel_rows
                .into_iter()
                .filter(|channel_row| channel_row.channel_config_encrypted.is_none())
            {
                let (stored, encrypted) =
                    q::prepare_channel_config(encryption, &channel_row.channel_config)
                        .map_err(classify_anyhow)?;
                let input = UpdateAppChannel {
                    channel_config: Some(stored),
                    channel_config_encrypted: encrypted,
                    ..Default::default()
                };
                ctx.db
                    .update_app_channel(channel_row.id, input)
                    .await
                    .map_err(classify_anyhow)?;
            }
        }

        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        if req.status.is_some() {
            sync_all_schedule_bindings(ctx, &app).await?;
        }
        Ok(redact_app_for_response(app))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAppCmd>() }

// ============================================================================
// DeleteApp
// ============================================================================

/// Archive an app (soft delete).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteApp {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for DeleteApp {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_app",
            category: "apps",
            description: "Archive an app (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/apps/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        ctx.db
            .delete_app(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;

        if let Some(row) = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
        {
            let app = q::row_to_app(&ctx.db, ctx.encryption.as_ref(), row, ctx.org_id()).await;
            sync_all_schedule_bindings(ctx, &app).await?;
        }

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteApp>() }

// ============================================================================
// DestroyApp (hard delete)
// ============================================================================

/// Permanently delete an archived app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DestroyApp {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for DestroyApp {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_app",
            category: "apps",
            description: "Permanently delete an archived app.",
            method: "POST",
            path: "/v1/apps/{id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if existing.status != "archived" {
            return Err(CommandError::bad_request(
                "App must be archived before deletion",
            ));
        }

        for channel_row in ctx
            .db
            .list_app_channels(existing.id)
            .await
            .map_err(classify_anyhow)?
        {
            remove_schedule_binding_for_channel(ctx, &channel_row).await?;
        }

        ctx.db
            .destroy_app(ctx.org_id(), existing.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"destroyed": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyApp>() }

// ============================================================================
// PublishApp
// ============================================================================

/// Publish an app (start accepting requests).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PublishApp {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for PublishApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "publish_app",
            category: "apps",
            description: "Publish an app (start accepting requests).",
            method: "POST",
            path: "/v1/apps/{id}/publish",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        if existing.status != "draft" {
            return Err(CommandError::bad_request(
                "App must be draft before publishing",
            ));
        }
        require_app_channel_before_publish(ctx, existing.id).await?;

        let input = UpdateApp {
            status: Some("published".to_string()),
            published_at: UpdateField::Set(Utc::now()),
            ..Default::default()
        };

        let encryption = ctx.encryption.as_ref();
        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        sync_all_schedule_bindings(ctx, &app).await?;
        Ok(redact_app_for_response(app))
    }
}

inventory::submit! { CommandDescriptor::of::<PublishApp>() }

// ============================================================================
// UnpublishApp
// ============================================================================

/// Unpublish an app (stop accepting requests).
#[derive(Debug, Deserialize, ToSchema)]
pub struct UnpublishApp {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

impl Command for UnpublishApp {
    type Output = App;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "unpublish_app",
            category: "apps",
            description: "Unpublish an app (stop accepting requests).",
            method: "POST",
            path: "/v1/apps/{id}/unpublish",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<App, CommandError> {
        let app_id: AppId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let existing = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        let input = UpdateApp {
            status: Some("draft".to_string()),
            ..Default::default()
        };

        let encryption = ctx.encryption.as_ref();
        let row = ctx
            .db
            .update_app(ctx.org_id(), existing.id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        let app = q::row_to_app(&ctx.db, encryption, row, ctx.org_id()).await;
        sync_all_schedule_bindings(ctx, &app).await?;
        Ok(redact_app_for_response(app))
    }
}

inventory::submit! { CommandDescriptor::of::<UnpublishApp>() }

// ============================================================================
// AddChannel
// ============================================================================

/// Add a channel to an app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddChannel {
    /// App's prefixed public identifier.
    pub app_id: String,
    #[serde(flatten)]
    pub req: AddChannelRequest,
}

impl Command for AddChannel {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_app_channel",
            category: "apps",
            description: "Add a channel to an app.",
            method: "POST",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        ensure_channel_type_enabled(ctx, &self.req.channel_type)?;
        reject_new_schedule_channel(&self.req.channel_type)?;

        if self.req.channel_type == ChannelType::Schedule {
            let _ = durable_store(ctx)?;
        }

        let channel_config = normalize_and_validate_channel_config(
            self.req.channel_type.clone(),
            self.req.channel_config.unwrap_or_default(),
        )?;
        let (stored_plaintext, encrypted) =
            q::prepare_channel_config(encryption, &channel_config).map_err(classify_anyhow)?;

        let channel_uuid = Uuid::now_v7();
        let channel_public_id = AppChannelId::from_uuid(channel_uuid);
        let input = CreateAppChannelRow {
            public_id: channel_public_id.to_string(),
            channel_type: self.req.channel_type.to_string(),
            channel_config: stored_plaintext,
            channel_config_encrypted: encrypted,
            durable_schedule_id: None,
            enabled: self.req.enabled.unwrap_or(true),
        };

        let row = if self.req.channel_type == ChannelType::Schedule && input.enabled {
            ctx.db
                .create_app_channel_enforcing_schedule_cap(
                    ctx.org_id(),
                    app.id,
                    input,
                    schedule_channel_max_per_org(),
                )
                .await
                .map_err(classify_anyhow)?
        } else {
            ctx.db
                .create_app_channel(app.id, input)
                .await
                .map_err(classify_anyhow)?
        };

        let app = q::row_to_app(&ctx.db, encryption, app, ctx.org_id()).await;
        sync_schedule_binding_for_channel(ctx, &app, &row).await?;

        Ok(redact_channel_for_response(q::channel_row_to_channel(
            encryption, row,
        )))
    }
}

inventory::submit! { CommandDescriptor::of::<AddChannel>() }

// ============================================================================
// AddScheduleChannel
// ============================================================================

/// Add a schedule invocation channel to an app using flat args suitable for bash mode.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddScheduleChannelCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    /// Cron expression. Accepts 5 fields (`*/10 * * * *`) or 7 fields
    /// (`0 */10 * * * * *`); 5-field input is stored as 7-field cron.
    pub cron_expression: String,
    pub timezone: Option<String>,
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    pub message: String,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: Option<bool>,
}

impl Command for AddScheduleChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_schedule_app_channel",
            category: "apps",
            description: "Add a schedule invocation channel to an app using flat args. cron_expression accepts 5-field or 7-field cron; 5-field input is normalized to 7-field cron.",
            method: "POST",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        AddChannel {
            app_id: self.app_id,
            req: AddChannelRequest {
                channel_type: ChannelType::Schedule,
                channel_config: Some(json!({
                    "cron_expression": self.cron_expression,
                    "timezone": self.timezone.unwrap_or_else(|| "UTC".to_string()),
                    "session_mode": self.session_mode,
                    "message": self.message,
                })),
                enabled: self.enabled,
            },
        }
        .execute(ctx)
        .await
    }
}

inventory::submit! { CommandDescriptor::of::<AddScheduleChannelCmd>() }

// ============================================================================
// TriggerAppScheduleChannel
// ============================================================================

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct TriggerAppScheduleChannelOutput {
    /// Session's prefixed public identifier.
    pub session_id: SessionId,
    pub created_session: bool,
}

/// Manually trigger an app schedule channel, primarily for testing.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TriggerAppScheduleChannelCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    /// Channel's prefixed public identifier.
    pub channel_id: String,
}

impl Command for TriggerAppScheduleChannelCmd {
    type Output = TriggerAppScheduleChannelOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "trigger_app_schedule_channel",
            category: "apps",
            description: "Manually trigger an app schedule channel. Use app_id and channel_id from list_app_channels.",
            method: "POST",
            path: "/v1/apps/{id}/channels/{channel_id}/trigger",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<TriggerAppScheduleChannelOutput, CommandError> {
        let session_service = ctx.session_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Session service not available"))
        })?;
        let message_service = ctx.message_service.as_ref().ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Message service not available"))
        })?;
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;
        let result = invoke_scheduled_app_channel(
            &ctx.db,
            ctx.encryption.as_ref(),
            session_service,
            message_service,
            ctx.org_id(),
            &app_id.to_string(),
            &self.channel_id,
        )
        .await?;
        Ok(TriggerAppScheduleChannelOutput {
            session_id: result.session_id,
            created_session: result.created_session,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<TriggerAppScheduleChannelCmd>() }

// ============================================================================
// AddWebhookChannel
// ============================================================================

/// Add a webhook invocation channel to an app using flat args suitable for bash mode.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddWebhookChannelCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    pub token: String,
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    pub message: String,
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: Option<bool>,
}

impl Command for AddWebhookChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_webhook_app_channel",
            category: "apps",
            description: "Add a webhook invocation channel to an app using flat args.",
            method: "POST",
            path: "/v1/apps/{id}/channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        AddChannel {
            app_id: self.app_id,
            req: AddChannelRequest {
                channel_type: ChannelType::Webhook,
                channel_config: Some(json!({
                    "token": self.token,
                    "session_mode": self.session_mode,
                    "message": self.message,
                })),
                enabled: self.enabled,
            },
        }
        .execute(ctx)
        .await
    }
}

inventory::submit! { CommandDescriptor::of::<AddWebhookChannelCmd>() }

// ============================================================================
// AddA2aChannel
// ============================================================================

/// Generate a fresh A2A API key, return (plaintext, sha256_hash, display_prefix).
///
/// Plaintext format: `evra2a_<64 hex chars>` — 32 random bytes (256-bit
/// entropy). The hash is SHA-256 hex, matching `auth/api_key.rs`. The prefix
/// is the first 8 hex chars after `evra2a_`, suffixed with `...`, for
/// non-secret UI display.
pub fn generate_a2a_api_key() -> (String, String, String) {
    use rand::Rng;

    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let hex = hex::encode(bytes);
    let plaintext = format!("evra2a_{hex}");
    let hash = hash_a2a_api_key(&plaintext);
    let prefix = format!("evra2a_{}...", &hex[..8]);
    (plaintext, hash, prefix)
}

/// Hash a plaintext A2A API key using SHA-256, returning hex.
pub fn hash_a2a_api_key(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

/// Output of [`AddA2aChannelCmd`] — includes the plaintext API key (returned
/// **once**, never persisted) plus the resulting [`AppChannel`].
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct AddA2aChannelOutput {
    /// Plaintext API key. Persist this — it cannot be recovered later.
    pub api_key: String,
    /// The created A2A channel.
    pub channel: AppChannel,
}

/// Add an A2A invocation channel to an app. Generates the API key server-side
/// and returns the plaintext exactly once.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddA2aChannelCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    pub message: String,
    pub agent_card_name: Option<String>,
    pub agent_card_description: Option<String>,
    #[serde(default)]
    pub auth: Option<AppEndpointAuthConfig>,
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: Option<bool>,
}

impl Command for AddA2aChannelCmd {
    type Output = AddA2aChannelOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_a2a_app_channel",
            category: "apps",
            description: "Add an A2A (Agent2Agent) invocation channel to an app. Returns the plaintext API key exactly once.",
            method: "POST",
            path: "/v1/apps/{id}/a2a-channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AddA2aChannelOutput, CommandError> {
        let (plaintext, hash, prefix) = generate_a2a_api_key();
        let mut config = json!({
            "api_key_hash": hash,
            "api_key_prefix": prefix,
            "session_mode": self.session_mode,
            "message": self.message,
        });
        if let Some(name) = self.agent_card_name {
            config["agent_card_name"] = Value::String(name);
        }
        if let Some(desc) = self.agent_card_description {
            config["agent_card_description"] = Value::String(desc);
        }
        if let Some(auth) = self.auth {
            config["auth"] = serde_json::to_value(auth).map_err(|e| {
                CommandError::bad_request(format!("Invalid app endpoint auth config: {e}"))
            })?;
        }
        let channel = AddChannel {
            app_id: self.app_id,
            req: AddChannelRequest {
                channel_type: ChannelType::A2a,
                channel_config: Some(config),
                enabled: self.enabled,
            },
        }
        .execute(ctx)
        .await?;
        Ok(AddA2aChannelOutput {
            api_key: plaintext,
            channel,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<AddA2aChannelCmd>() }

// ============================================================================
// RegenerateA2aApiKey
// ============================================================================

/// Regenerate the API key for an A2A channel. Returns the new plaintext key
/// exactly once and invalidates the previous key.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegenerateA2aApiKeyCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    /// Channel's prefixed public identifier.
    pub channel_id: String,
}

/// Output of [`RegenerateA2aApiKeyCmd`] — includes the newly generated
/// plaintext API key (returned **once**, never persisted) plus the updated
/// [`AppChannel`].
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct RegenerateA2aApiKeyOutput {
    /// New plaintext API key. Persist this — it cannot be recovered later.
    /// The previous key is invalidated immediately.
    pub api_key: String,
    /// The updated A2A channel.
    pub channel: AppChannel,
}

impl Command for RegenerateA2aApiKeyCmd {
    type Output = RegenerateA2aApiKeyOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "regenerate_a2a_app_channel_key",
            category: "apps",
            description: "Regenerate the A2A channel API key. Returns the new plaintext exactly once and invalidates the previous key.",
            method: "POST",
            path: "/v1/apps/{id}/a2a-channels/{channel_id}/regenerate-key",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<RegenerateA2aApiKeyOutput, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }
        if channel_row.channel_type != ChannelType::A2a.to_string() {
            return Err(CommandError::bad_request("Channel is not an A2A channel"));
        }

        let encryption = ctx.encryption.as_ref();
        let mut existing_config: Value = q::decrypt_channel_config(
            encryption,
            channel_row.channel_config_encrypted.as_deref(),
            &channel_row.channel_config,
        );
        let (plaintext, hash, prefix) = generate_a2a_api_key();
        if let Some(map) = existing_config.as_object_mut() {
            map.insert("api_key_hash".to_string(), Value::String(hash));
            map.insert("api_key_prefix".to_string(), Value::String(prefix));
        } else {
            return Err(CommandError::bad_request(
                "Existing A2A channel configuration is not a JSON object",
            ));
        }
        let existing_config =
            normalize_and_validate_channel_config(ChannelType::A2a, existing_config)?;

        let (stored, encrypted) =
            q::prepare_channel_config(encryption, &existing_config).map_err(classify_anyhow)?;
        let row = ctx
            .db
            .update_app_channel(
                channel_row.id,
                UpdateAppChannel {
                    channel_config: Some(stored),
                    channel_config_encrypted: encrypted,
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        Ok(RegenerateA2aApiKeyOutput {
            api_key: plaintext,
            channel: redact_channel_for_response(q::channel_row_to_channel(encryption, row)),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<RegenerateA2aApiKeyCmd>() }

// ============================================================================
// AddApiEndpointChannel
// ============================================================================

/// Generate a fresh api_endpoint execution API key, returning
/// (plaintext, sha256_hash, display_prefix).
///
/// Plaintext format: `evr_app_<64 hex chars>` — 32 random bytes (256-bit
/// entropy), prefix-scoped so secret scanners can target it distinctly from
/// `evr_pat_` (personal access tokens) and `evra2a_` (A2A keys). The hash is
/// SHA-256 hex; the prefix is the first 8 hex chars after `evr_app_`, suffixed
/// with `...`, for non-secret UI display.
pub fn generate_app_api_key() -> (String, String, String) {
    use rand::Rng;

    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let hex = hex::encode(bytes);
    let plaintext = format!("evr_app_{hex}");
    let hash = hash_app_api_key(&plaintext);
    let prefix = format!("evr_app_{}...", &hex[..8]);
    (plaintext, hash, prefix)
}

/// Hash a plaintext api_endpoint API key using SHA-256, returning hex.
pub fn hash_app_api_key(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plaintext.as_bytes()))
}

/// Output of [`AddApiEndpointChannelCmd`] — includes the plaintext API key
/// (returned **once**, never persisted) plus the resulting [`AppChannel`].
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct AddApiEndpointChannelOutput {
    /// Plaintext API key. Persist this — it cannot be recovered later.
    pub api_key: String,
    /// The created api_endpoint channel.
    pub channel: AppChannel,
}

/// Add an api_endpoint (execution-only API key) channel to an app. Generates
/// the API key server-side and returns the plaintext exactly once.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddApiEndpointChannelCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    #[serde(default)]
    pub session_mode: InvocationSessionMode,
    #[serde(default)]
    pub auth: Option<AppEndpointAuthConfig>,
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    /// Whether this resource is enabled.
    pub enabled: Option<bool>,
}

impl Command for AddApiEndpointChannelCmd {
    type Output = AddApiEndpointChannelOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "add_api_endpoint_app_channel",
            category: "apps",
            description: "Add an api_endpoint (execution-only API key) channel to an app. Returns the plaintext API key exactly once.",
            method: "POST",
            path: "/v1/apps/{id}/api-endpoint-channels",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AddApiEndpointChannelOutput, CommandError> {
        let (plaintext, hash, prefix) = generate_app_api_key();
        let mut config = json!({
            "api_key_hash": hash,
            "api_key_prefix": prefix,
            "session_mode": self.session_mode,
        });
        if let Some(auth) = self.auth {
            config["auth"] = serde_json::to_value(auth).map_err(|e| {
                CommandError::bad_request(format!("Invalid app endpoint auth config: {e}"))
            })?;
        }
        let channel = AddChannel {
            app_id: self.app_id,
            req: AddChannelRequest {
                channel_type: ChannelType::ApiEndpoint,
                channel_config: Some(config),
                enabled: self.enabled,
            },
        }
        .execute(ctx)
        .await?;
        Ok(AddApiEndpointChannelOutput {
            api_key: plaintext,
            channel,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<AddApiEndpointChannelCmd>() }

// ============================================================================
// RegenerateApiEndpointApiKey
// ============================================================================

/// Regenerate the API key for an api_endpoint channel. Returns the new
/// plaintext key exactly once and invalidates the previous key.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegenerateApiEndpointApiKeyCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    /// Channel's prefixed public identifier.
    pub channel_id: String,
}

/// Output of [`RegenerateApiEndpointApiKeyCmd`] — includes the newly generated
/// plaintext API key (returned **once**, never persisted) plus the updated
/// [`AppChannel`].
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct RegenerateApiEndpointApiKeyOutput {
    /// New plaintext API key. Persist this — it cannot be recovered later.
    /// The previous key is invalidated immediately.
    pub api_key: String,
    /// The updated api_endpoint channel.
    pub channel: AppChannel,
}

impl Command for RegenerateApiEndpointApiKeyCmd {
    type Output = RegenerateApiEndpointApiKeyOutput;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "regenerate_api_endpoint_app_channel_key",
            category: "apps",
            description: "Regenerate the api_endpoint channel API key. Returns the new plaintext exactly once and invalidates the previous key.",
            method: "POST",
            path: "/v1/apps/{id}/api-endpoint-channels/{channel_id}/regenerate-key",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<RegenerateApiEndpointApiKeyOutput, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;
        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;
        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }
        if channel_row.channel_type != ChannelType::ApiEndpoint.to_string() {
            return Err(CommandError::bad_request(
                "Channel is not an api_endpoint channel",
            ));
        }

        let encryption = ctx.encryption.as_ref();
        let mut existing_config: Value = q::decrypt_channel_config(
            encryption,
            channel_row.channel_config_encrypted.as_deref(),
            &channel_row.channel_config,
        );
        let (plaintext, hash, prefix) = generate_app_api_key();
        if let Some(map) = existing_config.as_object_mut() {
            map.insert("api_key_hash".to_string(), Value::String(hash));
            map.insert("api_key_prefix".to_string(), Value::String(prefix));
        } else {
            return Err(CommandError::bad_request(
                "Existing api_endpoint channel configuration is not a JSON object",
            ));
        }
        let existing_config =
            normalize_and_validate_channel_config(ChannelType::ApiEndpoint, existing_config)?;

        let (stored, encrypted) =
            q::prepare_channel_config(encryption, &existing_config).map_err(classify_anyhow)?;
        let row = ctx
            .db
            .update_app_channel(
                channel_row.id,
                UpdateAppChannel {
                    channel_config: Some(stored),
                    channel_config_encrypted: encrypted,
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        Ok(RegenerateApiEndpointApiKeyOutput {
            api_key: plaintext,
            channel: redact_channel_for_response(q::channel_row_to_channel(encryption, row)),
        })
    }
}

inventory::submit! { CommandDescriptor::of::<RegenerateApiEndpointApiKeyCmd>() }

// ============================================================================
// UpdateChannel
// ============================================================================

/// Update a channel on an app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateChannelCmd {
    /// App's prefixed public identifier.
    pub app_id: String,
    /// Channel's prefixed public identifier.
    pub channel_id: String,
    #[serde(flatten)]
    pub req: UpdateChannelRequest,
}

impl Command for UpdateChannelCmd {
    type Output = AppChannel;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_app_channel",
            category: "apps",
            description: "Update a channel on an app.",
            method: "PATCH",
            path: "/v1/apps/{id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AppChannel, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let encryption = ctx.encryption.as_ref();
        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }

        let current_channel_type = ChannelType::from_str_opt(&channel_row.channel_type)
            .ok_or_else(|| CommandError::bad_request("Unknown existing channel type"))?;
        let final_channel_type = self
            .req
            .channel_type
            .clone()
            .unwrap_or(current_channel_type.clone());
        reject_new_schedule_channel(&final_channel_type)?;
        let existing_decrypted = q::decrypt_channel_config(
            encryption,
            channel_row.channel_config_encrypted.as_deref(),
            &channel_row.channel_config,
        );
        let mut final_channel_config = self
            .req
            .channel_config
            .clone()
            .unwrap_or_else(|| existing_decrypted.clone());
        // Secret-bearing channel fields are write-only. Preserve existing
        // secret material when a PATCH only sends user-editable fields.
        merge_preserved_secret_fields(
            final_channel_type.clone(),
            &mut final_channel_config,
            &existing_decrypted,
        );
        let enforce_schedule_cap = if final_channel_type == ChannelType::Schedule {
            let _ = durable_store(ctx)?;
            let final_enabled = self.req.enabled.unwrap_or(channel_row.enabled);
            let was_enabled_schedule =
                current_channel_type == ChannelType::Schedule && channel_row.enabled;
            final_enabled && !was_enabled_schedule
        } else {
            false
        };
        ensure_channel_type_enabled(ctx, &final_channel_type)?;
        let normalized_channel_config =
            normalize_and_validate_channel_config(final_channel_type, final_channel_config)?;

        let config_changed = self.req.channel_config.is_some();
        let (channel_config, channel_config_encrypted) = if config_changed {
            // Persist the merged config (so A2A preserves the existing
            // api_key_hash / prefix when the client omits them).
            let (stored, encrypted) =
                q::prepare_channel_config(encryption, &normalized_channel_config)
                    .map_err(classify_anyhow)?;
            (Some(stored), encrypted)
        } else {
            (None, None)
        };

        let input = UpdateAppChannel {
            channel_type: self.req.channel_type.map(|ct| ct.to_string()),
            channel_config,
            channel_config_encrypted,
            durable_schedule_id: UpdateField::Unchanged,
            enabled: self.req.enabled,
        };

        let row = if enforce_schedule_cap {
            ctx.db
                .update_app_channel_enforcing_schedule_cap(
                    ctx.org_id(),
                    channel_row.id,
                    input,
                    schedule_channel_max_per_org(),
                )
                .await
                .map_err(classify_anyhow)?
        } else {
            ctx.db
                .update_app_channel(channel_row.id, input)
                .await
                .map_err(classify_anyhow)?
        }
        .ok_or_else(|| CommandError::not_found("Channel"))?;

        let app = q::row_to_app(&ctx.db, encryption, app, ctx.org_id()).await;
        sync_schedule_binding_for_channel(ctx, &app, &row).await?;

        Ok(redact_channel_for_response(q::channel_row_to_channel(
            encryption, row,
        )))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateChannelCmd>() }

// ============================================================================
// DeleteChannel
// ============================================================================

/// Remove a channel from an app.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteChannel {
    /// App's prefixed public identifier.
    pub app_id: String,
    /// Channel's prefixed public identifier.
    pub channel_id: String,
}

impl Command for DeleteChannel {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_app_channel",
            category: "apps",
            description: "Remove a channel from an app.",
            method: "DELETE",
            path: "/v1/apps/{id}/channels/{channel_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&APP_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let app_id: AppId = self
            .app_id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid app ID: {e}")))?;

        let app = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &app_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        if !matches!(app.status.as_str(), "draft" | "published") {
            return Err(CommandError::bad_request(
                "Archived or deleted apps cannot be edited",
            ));
        }

        let channel_row = ctx
            .db
            .get_app_channel_by_public_id(&self.channel_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Channel"))?;

        if channel_row.app_id != app.id {
            return Err(CommandError::bad_request(
                "Channel does not belong to this app",
            ));
        }

        remove_schedule_binding_for_channel(ctx, &channel_row).await?;

        ctx.db
            .delete_app_channel(channel_row.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteChannel>() }

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::typed_id::{AppChannelId, HarnessId, PrincipalId};

    // Serialize env-mutating tests to prevent concurrent remove_var / read_var races.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn apps_require_an_agent_for_writes() {
        let err = require_app_agent(None).expect_err("agent-less writes must be rejected");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
        assert!(
            err.to_string()
                .contains("Existing agent-less apps remain runnable")
        );
    }

    #[test]
    fn new_schedule_channels_point_to_agent_triggers() {
        let err = reject_new_schedule_channel(&ChannelType::Schedule)
            .expect_err("new App schedule channels must be rejected");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
        assert!(err.to_string().contains("app's agent"));
    }

    #[test]
    fn non_schedule_channels_remain_available() {
        for channel_type in [
            ChannelType::Slack,
            ChannelType::AgUi,
            ChannelType::Webhook,
            ChannelType::A2a,
            ChannelType::Fcp,
            ChannelType::ApiEndpoint,
            ChannelType::PublicChat,
        ] {
            reject_new_schedule_channel(&channel_type)
                .unwrap_or_else(|err| panic!("{channel_type} unexpectedly rejected: {err}"));
        }
    }

    fn test_app_and_channel() -> (App, AppChannel) {
        let now = Utc::now();
        let channel = AppChannel {
            public_id: AppChannelId::from_seed(2),
            internal_id: Uuid::nil(),
            channel_type: ChannelType::Webhook,
            channel_config: json!({}),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        let app = App {
            public_id: AppId::from_seed(1),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "Test App".to_string(),
            description: None,
            harness_id: HarnessId::from_seed(3),
            agent_id: None,
            agent_version_policy: everruns_platform::AgentVersionPolicy::Default,
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(4),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            channels: vec![channel.clone()],
            status: AppStatus::Published,
            published_at: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };

        (app, channel)
    }

    #[test]
    fn app_invocation_message_metadata_includes_system_app_id() {
        let (app, channel) = test_app_and_channel();

        let metadata =
            app_invocation_message_metadata(&app, &channel, AppInvocationSource::Webhook);

        assert_eq!(
            metadata.get("_app_id"),
            Some(&Value::String(app.public_id.to_string()))
        );
        assert_eq!(
            metadata.get("app_channel_id"),
            Some(&Value::String(channel.public_id.to_string()))
        );
        // The bare `app_id` key was previously emitted alongside `_app_id`
        // but had no consumer; the metadata bag uses the same `_app_id`
        // convention as the AG-UI and Slack ingress paths.
        assert!(!metadata.contains_key("app_id"));
    }

    #[test]
    fn parse_run_history_window_rejects_unicode_unit() {
        let err = parse_run_history_window(Some("1µ")).expect_err("unicode unit should fail");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn parse_run_history_window_rejects_overflowing_amount() {
        let err = parse_run_history_window(Some("9223372036854775807d"))
            .expect_err("overflowing window should fail");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    // ---- schedule channel validation -----------------------------------------

    #[test]
    fn schedule_min_interval_default_is_300() {
        let _lock = lock_env();
        // Safety: ENV_LOCK serializes all env-mutating tests in this module.
        unsafe { std::env::remove_var("SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS") };
        assert_eq!(schedule_channel_min_interval_seconds(), 300);
    }

    #[test]
    fn cron_min_interval_every_5_min() {
        // "0 */5 * * * *" fires every 5 minutes = 300 s.
        let schedule = cron::Schedule::from_str("0 */5 * * * *").expect("valid cron");
        let interval = cron_min_interval_seconds(&schedule, 1).expect("interval exists");
        assert_eq!(interval, 300);
    }

    #[test]
    fn cron_min_interval_every_minute() {
        // "0 * * * * *" fires every 60 s.
        let schedule = cron::Schedule::from_str("0 * * * * *").expect("valid cron");
        let interval = cron_min_interval_seconds(&schedule, 1).expect("interval exists");
        assert_eq!(interval, 60);
    }

    #[test]
    fn cron_min_interval_detects_later_non_uniform_burst() {
        let schedule =
            cron::Schedule::from_str("0 0,6,12,18,24,30,36,42,48,54,55,56,57,58,59 * * * * *")
                .expect("valid cron");
        let interval = cron_min_interval_seconds(&schedule, 1).expect("interval exists");
        assert_eq!(interval, 60);
    }

    #[test]
    fn cron_min_interval_detects_future_year_burst() {
        let schedule = cron::Schedule::from_str("* * * 1 1 * 2029").expect("valid cron");
        let interval = cron_min_interval_seconds(&schedule, 300).expect("interval exists");
        assert_eq!(interval, 1);
    }

    #[test]
    fn cron_min_interval_handles_single_horizon_occurrence() {
        // A once-yearly schedule has exactly one occurrence inside the 366-day
        // horizon, so it takes the fallback path. The fallback must measure the
        // ~1-year cadence across the sampled occurrences, not compare the single
        // horizon occurrence against itself (which would yield a spurious 0 and
        // falsely reject a valid sparse schedule).
        let schedule = cron::Schedule::from_str("0 0 0 1 7 * *").expect("valid cron");
        let interval = cron_min_interval_seconds(&schedule, 300).expect("interval exists");
        assert!(
            interval > 300,
            "a once-yearly schedule must not report a sub-limit burst, got {interval}"
        );
    }

    #[test]
    fn schedule_channel_rejects_too_frequent_cron() {
        // "0 * * * * *" = every minute (60 s) < 300 s default limit.
        let config = json!({
            "cron_expression": "* * * * *",
            "message": "tick"
        });
        let err = normalize_and_validate_channel_config(ChannelType::Schedule, config)
            .expect_err("should reject sub-5-min cron");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn schedule_channel_rejects_future_year_burst() {
        let config = json!({
            "cron_expression": "* * * 1 1 * 2029",
            "message": "future burst"
        });
        let err = normalize_and_validate_channel_config(ChannelType::Schedule, config)
            .expect_err("should reject future burst below interval limit");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn schedule_channel_accepts_5_min_cron() {
        let _lock = lock_env();
        // "*/5 * * * *" = every 5 minutes (300 s) — at the default limit.
        // Safety: ENV_LOCK serializes all env-mutating tests in this module.
        unsafe { std::env::remove_var("SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS") };
        let config = json!({
            "cron_expression": "*/5 * * * *",
            "message": "ping"
        });
        normalize_and_validate_channel_config(ChannelType::Schedule, config)
            .expect("5-min cron should be accepted");
    }

    #[test]
    fn schedule_channel_rejects_empty_message() {
        let config = json!({
            "cron_expression": "0 */5 * * * *",
            "message": "   "
        });
        let err = normalize_and_validate_channel_config(ChannelType::Schedule, config)
            .expect_err("empty message should fail");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn schedule_min_interval_rejects_zero_env() {
        let _lock = lock_env();
        // Safety: ENV_LOCK serializes all env-mutating tests in this module.
        unsafe { std::env::set_var("SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS", "0") };
        assert_eq!(
            schedule_channel_min_interval_seconds(),
            300,
            "zero should fall back to default"
        );
        unsafe { std::env::remove_var("SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS") };
    }

    #[test]
    fn schedule_max_per_org_rejects_zero_env() {
        let _lock = lock_env();
        // Safety: ENV_LOCK serializes all env-mutating tests in this module.
        unsafe { std::env::set_var("SCHEDULE_CHANNEL_MAX_PER_ORG", "0") };
        assert_eq!(
            schedule_channel_max_per_org(),
            10,
            "zero should fall back to default"
        );
        unsafe { std::env::remove_var("SCHEDULE_CHANNEL_MAX_PER_ORG") };
    }

    #[test]
    fn public_chat_channel_accepts_anonymous_default_config() {
        normalize_and_validate_channel_config(ChannelType::PublicChat, json!({}))
            .expect("empty public_chat config defaults to anonymous and is valid");
    }

    #[test]
    fn public_chat_channel_accepts_google_oidc_and_branding() {
        let config = json!({
            "anonymous": false,
            "auth": {
                "mode": "google_oidc",
                "provider": {"type": "google_oidc", "client_id": "abc.apps.googleusercontent.com"}
            },
            "branding": {"display_name": "Helpdesk", "primary_color": "#0A1636"},
            "captcha": {"site_key": "1x0000AA", "secret_key": "1x0000secret"}
        });
        normalize_and_validate_channel_config(ChannelType::PublicChat, config)
            .expect("google oidc + branding + captcha is valid");
    }

    #[test]
    fn public_chat_channel_rejects_empty_token() {
        let err =
            normalize_and_validate_channel_config(ChannelType::PublicChat, json!({"token": "   "}))
                .expect_err("whitespace-only token should fail");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn public_chat_channel_rejects_enabled_captcha_without_secret() {
        // captcha.enabled defaults to true; an enabled captcha with no secret
        // would fail closed at runtime, so it must be rejected at write time.
        let err = normalize_and_validate_channel_config(
            ChannelType::PublicChat,
            json!({"captcha": {"site_key": "k"}}),
        )
        .expect_err("enabled captcha without secret should fail");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn public_chat_channel_allows_disabled_captcha_without_secret() {
        normalize_and_validate_channel_config(
            ChannelType::PublicChat,
            json!({"captcha": {"enabled": false, "site_key": "k"}}),
        )
        .expect("disabled captcha without secret is allowed");
    }

    #[test]
    fn public_chat_channel_rejects_captcha_without_site_key() {
        let err = normalize_and_validate_channel_config(
            ChannelType::PublicChat,
            json!({"captcha": {"site_key": "", "secret_key": "s"}}),
        )
        .expect_err("captcha without site_key should fail");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn public_chat_channel_rejects_overlong_rate_limit() {
        let err = normalize_and_validate_channel_config(
            ChannelType::PublicChat,
            json!({"rate_limit_per_minute": 2_000_000}),
        )
        .expect_err("rate limit above cap should fail");
        assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
    }

    #[test]
    fn public_chat_redacts_token_and_captcha_secret() {
        let mut config = json!({
            "token": "shared-secret",
            "captcha": {"provider": "turnstile", "site_key": "1x0000AA", "secret_key": "1x0000secret"}
        });
        redact_channel_config(&ChannelType::PublicChat, &mut config);
        assert!(config.get("token").is_none());
        assert_eq!(config.get("token_configured"), Some(&Value::Bool(true)));
        let captcha = config.get("captcha").and_then(Value::as_object).unwrap();
        // Site key stays (public, needed by the client widget); secret is gone.
        assert_eq!(
            captcha.get("site_key").and_then(Value::as_str),
            Some("1x0000AA")
        );
        assert!(captcha.get("secret_key").is_none());
        assert_eq!(
            captcha.get("secret_key_configured"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn public_chat_preserves_captcha_secret_on_patch() {
        // PATCH that toggles `enabled` but omits the write-only secret must keep
        // the stored secret rather than silently disabling verification.
        let mut final_config = json!({
            "captcha": {"provider": "turnstile", "enabled": false, "site_key": "1x0000AA"}
        });
        let existing = json!({
            "captcha": {"provider": "turnstile", "enabled": true, "site_key": "1x0000AA", "secret_key": "1x0000secret"}
        });
        merge_preserved_secret_fields(ChannelType::PublicChat, &mut final_config, &existing);
        let captcha = final_config
            .get("captcha")
            .and_then(Value::as_object)
            .unwrap();
        assert_eq!(
            captcha.get("secret_key").and_then(Value::as_str),
            Some("1x0000secret")
        );
    }
}
