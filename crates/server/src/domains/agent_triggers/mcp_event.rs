//! MCP event triggers: inbound MCP Events (EVE-1121) as an agent trigger
//! source. See `knowledge/integrations/mcp-events.md` (spec revision pinned
//! there) and `knowledge/runtime-resources/agent-triggers.md`.
//!
//! Design decisions:
//! - **The agent subscribes as itself.** The trigger names one of the agent's
//!   MCP server attachments (agent or harness layer). Everruns calls
//!   `events/subscribe` on it with the same transport and credential the agent
//!   uses for tools: a catalog preset with `actsAs: service` presents the
//!   agent identity's OAuth grant; a `user` attachment is refused, since a
//!   trigger has no user to act as.
//! - **One secret per trigger, generated here.** The `whsec_` secret is stored
//!   encrypted beside the subscription state, never in the trigger config, and
//!   never leaves Everruns except to the subscribed server.
//! - **The callback is the trigger's ingress**, `/v1/e/{ingress_id}/mcp-events`.
//!   Signature verification (Standard Webhooks, shared with the outbound half
//!   in `services::standard_webhooks`) is the only gate and runs before the
//!   body is parsed; then the timestamp window and, through the shared event
//!   pipeline, a replay check on `webhook-id`.
//! - **410 means "stop".** A delivery for a trigger that is gone, disabled,
//!   unsubscribed, flagged off, or for a superseded subscription id gets
//!   `410 Gone`, which the spec defines as ending the subscription. That is the
//!   safety net when an `events/unsubscribe` could not be delivered.
//! - **Refresh is periodic, not durable-workflow driven.** A background loop
//!   re-subscribes rows whose `refresh_before` is near. `events/subscribe` is
//!   idempotent on the server, so replicas racing on one row do no harm, and
//!   all state lives in the database, so any replica can pick it up.
//! - Everything after normalization (filter, dedupe, per-subject sessions,
//!   delivery log) is the shared pipeline in [`super::events`].

use super::events::{self, TriggerEvent, TriggerEventOutcome, TriggerEventRoute};
use super::types::{CreateAgentTriggerRequest, UpdateAgentTriggerRequest};
use crate::domains::agent_channels::invocation::render_message_template;
use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::domains::mcp_servers::McpServerService;
use crate::domains::mcp_servers::scoped_mcp;
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::records::{AgentTriggerType, McpEventTriggerConfig};
use crate::services::standard_webhooks::{self, SignedHeaders, VerifyError};
use crate::storage::StorageBackend;
use crate::storage::agent_trigger_mcp_subscriptions::{
    AgentTriggerMcpSubscriptionRow, MCP_SUBSCRIPTION_ACTIVE, MCP_SUBSCRIPTION_FAILED,
    MCP_SUBSCRIPTION_PENDING, UpsertAgentTriggerMcpSubscription,
};
use crate::storage::encryption::EncryptionService;
use crate::storage::models::{AgentRow, AgentTriggerRow};
use chrono::{DateTime, Utc};
use everruns_core::{EgressService, McpServerActsAs, McpServerAuthMode, ScopedMcpServer};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Path segment of the callback under the trigger's ingress.
pub const CALLBACK_SEGMENT: &str = "mcp-events";
/// Re-subscribe this long before the server's `refreshBefore`.
const REFRESH_MARGIN: chrono::Duration = chrono::Duration::minutes(15);
/// Assumed lifetime when a server omits `refreshBefore`.
const DEFAULT_LIFETIME: chrono::Duration = chrono::Duration::hours(1);
/// Wait after a failed refresh before the next attempt.
const RETRY_BACKOFF: chrono::Duration = chrono::Duration::minutes(5);
/// How often the refresher looks for due subscriptions.
const DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// Subscriptions refreshed per pass.
const REFRESH_BATCH: i64 = 100;
const MAX_NAME_LEN: usize = 200;
const MAX_ARGUMENTS_BYTES: usize = 8 * 1024;
const MAX_CURSOR_LEN: usize = 1024;
const MAX_CHALLENGE_LEN: usize = 512;

// ============================================================================
// Configuration
// ============================================================================

/// Build the stored config for a new MCP event trigger. The agent must have
/// an MCP server attachment by that name that the trigger can act through.
pub(super) async fn create_config(
    ctx: &Ctx,
    agent: &AgentRow,
    req: &CreateAgentTriggerRequest,
) -> Result<Value, CommandError> {
    require_flag(ctx)?;
    let config = McpEventTriggerConfig {
        server: require_name("mcp_server", req.mcp_server.as_deref())?,
        event: require_name("mcp_event", req.mcp_event.as_deref())?,
        arguments: normalize_arguments(req.mcp_event_arguments.clone())?,
        session_mode: req.session_mode,
        message: require_message(&req.message)?,
        subject_template: events::optional_template(req.subject_template.clone()),
        filter: events::optional_filter(req.filter.clone())?,
    };
    find_attachment(&ctx.db, agent, &config.server).await?;
    to_value(&config)
}

/// Apply a partial update. Returns the new config and whether the remote
/// subscription (server, event or arguments) changed.
pub(super) async fn update_config(
    ctx: &Ctx,
    agent: &AgentRow,
    trigger: &crate::records::AgentTrigger,
    req: &UpdateAgentTriggerRequest,
) -> Result<(Value, bool), CommandError> {
    let before = trigger
        .mcp_event_config()
        .map_err(|_| CommandError::bad_request("Invalid stored MCP event trigger configuration"))?;
    let mut config = before.clone();
    if let Some(server) = req.mcp_server.as_deref() {
        config.server = require_name("mcp_server", Some(server))?;
    }
    if let Some(event) = req.mcp_event.as_deref() {
        config.event = require_name("mcp_event", Some(event))?;
    }
    if let Some(arguments) = &req.mcp_event_arguments {
        config.arguments = normalize_arguments(Some(arguments.clone()))?;
    }
    if let Some(mode) = req.session_mode {
        config.session_mode = mode;
    }
    if let Some(message) = &req.message {
        config.message = require_message(message)?;
    }
    if let Some(template) = &req.subject_template {
        config.subject_template = events::optional_template(Some(template.clone()));
    }
    if let Some(filter) = &req.filter {
        config.filter = events::optional_filter(Some(filter.clone()))?;
    }
    events::validate_trigger_binding(config.session_mode, config.subject_template.is_some())?;
    let resubscribe = config.server != before.server
        || config.event != before.event
        || config.arguments != before.arguments;
    if resubscribe {
        require_flag(ctx)?;
        find_attachment(&ctx.db, agent, &config.server).await?;
    }
    Ok((to_value(&config)?, resubscribe))
}

fn require_flag(ctx: &Ctx) -> Result<(), CommandError> {
    if ctx.feature_flags.mcp_events {
        Ok(())
    } else {
        Err(CommandError::feature_not_enabled("mcp_events"))
    }
}

fn require_name(field: &str, value: Option<&str>) -> Result<String, CommandError> {
    let value = value.map(str::trim).unwrap_or_default();
    if value.is_empty() || value.len() > MAX_NAME_LEN {
        return Err(CommandError::bad_request(format!(
            "MCP event trigger requires {field} (1 to {MAX_NAME_LEN} characters)"
        )));
    }
    Ok(value.to_string())
}

fn require_message(message: &str) -> Result<String, CommandError> {
    if message.trim().is_empty() {
        return Err(CommandError::bad_request(
            "MCP event trigger requires a non-empty message",
        ));
    }
    Ok(message.to_string())
}

/// Subscription arguments are a JSON object (the event's `inputSchema`);
/// absent means `{}`.
fn normalize_arguments(arguments: Option<Value>) -> Result<Value, CommandError> {
    let arguments = match arguments {
        None | Some(Value::Null) => return Ok(json!({})),
        Some(arguments) => arguments,
    };
    if !arguments.is_object() {
        return Err(CommandError::bad_request(
            "mcp_event_arguments must be a JSON object",
        ));
    }
    if serde_json::to_vec(&arguments).map_or(0, |bytes| bytes.len()) > MAX_ARGUMENTS_BYTES {
        return Err(CommandError::bad_request(format!(
            "mcp_event_arguments may be at most {MAX_ARGUMENTS_BYTES} bytes"
        )));
    }
    Ok(arguments)
}

fn to_value(config: &McpEventTriggerConfig) -> Result<Value, CommandError> {
    serde_json::to_value(config).map_err(|e| CommandError::internal(e.into()))
}

/// The agent's MCP server attachment named `server`, from the harness and
/// agent layers. A `user` attachment cannot back a trigger.
async fn find_attachment(
    db: &Arc<StorageBackend>,
    agent_row: &AgentRow,
    server: &str,
) -> Result<(String, ScopedMcpServer), CommandError> {
    let agent = crate::domains::agents::queries::get_by_public_id(
        db,
        agent_row.org_id,
        &agent_row.public_id,
    )
    .await
    .map_err(classify_anyhow)?
    .ok_or_else(|| CommandError::not_found("Agent"))?;
    let harness = crate::domains::harnesses::queries::resolve_effective(
        db,
        agent_row.org_id,
        agent.harness_id,
    )
    .await
    .map_err(classify_anyhow)?;
    let (name, attachment) = scoped_mcp::merge_agent_scoped_mcp_servers(harness.as_ref(), &agent)
        .into_iter()
        .find(|(name, _)| name == server)
        .ok_or_else(|| {
            CommandError::bad_request(format!(
                "The agent has no MCP server named '{server}'. Attach it to the agent first."
            ))
        })?;
    if attachment.acts_as == McpServerActsAs::User {
        return Err(CommandError::bad_request(format!(
            "MCP server '{server}' acts as the calling user; an MCP event trigger runs as the \
             agent, so attach the server with actsAs: service"
        )));
    }
    Ok((name, attachment))
}

// ============================================================================
// Lifecycle hooks called by the trigger commands
// ============================================================================

fn service(ctx: &Ctx) -> Result<&Arc<McpEventTriggers>, CommandError> {
    ctx.mcp_event_triggers.as_ref().ok_or_else(|| {
        CommandError::unavailable("MCP event triggers are not available on this surface")
    })
}

/// After a trigger row is created: subscribe when it is enabled. A failed
/// subscribe removes the trigger again, so a create either fully works or
/// leaves nothing behind.
pub(super) async fn after_create(ctx: &Ctx, row: &AgentTriggerRow) -> Result<(), CommandError> {
    if !row.enabled {
        return Ok(());
    }
    let subscribed = match service(ctx) {
        Ok(service) => service.subscribe(row, true).await.map(|_| ()),
        Err(error) => Err(error),
    };
    if let Err(error) = subscribed {
        let _ = ctx.db.delete_agent_trigger_mcp_subscription(row.id).await;
        let _ = ctx.db.delete_agent_trigger(row.org_id, row.id).await;
        return Err(error);
    }
    Ok(())
}

/// After an update is stored: (re)subscribe, or unsubscribe, to match it.
/// A failed subscribe restores the previous config and enabled state.
pub(super) async fn after_update(
    ctx: &Ctx,
    before: &AgentTriggerRow,
    after: &AgentTriggerRow,
    resubscribe: bool,
) -> Result<(), CommandError> {
    if !after.enabled {
        if before.enabled {
            deactivate(ctx, before).await;
        }
        return Ok(());
    }
    let has_subscription = ctx
        .db
        .get_agent_trigger_mcp_subscription(after.id)
        .await
        .map_err(classify_anyhow)?
        .is_some();
    if before.enabled && has_subscription && !resubscribe {
        return Ok(());
    }
    let service = service(ctx)?;
    if before.enabled && resubscribe {
        service.unsubscribe(before).await;
    }
    if let Err(error) = service.subscribe(after, true).await {
        let _ = ctx.db.delete_agent_trigger_mcp_subscription(after.id).await;
        let restore = crate::storage::models::UpdateAgentTrigger {
            config: Some(before.config.clone()),
            enabled: Some(false),
            ..Default::default()
        };
        if let Err(restore_error) = ctx
            .db
            .update_agent_trigger(after.org_id, after.id, restore)
            .await
        {
            tracing::warn!(error = %restore_error, trigger_id = %after.id, "failed to restore MCP event trigger after a failed subscribe");
        }
        return Err(error);
    }
    Ok(())
}

/// Before a trigger is disabled or deleted: unsubscribe (best effort) and drop
/// the local subscription. Without the service (another surface), only the
/// local state is dropped; the callback then answers `410 Gone`.
pub(super) async fn deactivate(ctx: &Ctx, row: &AgentTriggerRow) {
    match ctx.mcp_event_triggers.as_ref() {
        Some(service) => service.unsubscribe(row).await,
        None => {
            let _ = ctx.db.delete_agent_trigger_mcp_subscription(row.id).await;
        }
    }
}

// ============================================================================
// Service: subscribe, refresh, unsubscribe, receive
// ============================================================================

/// A delivery as received on the callback route.
#[derive(Debug, Clone, Copy)]
pub struct InboundDelivery<'a> {
    pub ingress_id: &'a str,
    pub webhook_id: Option<&'a str>,
    pub timestamp: Option<&'a str>,
    pub signature: Option<&'a str>,
    pub subscription_id: Option<&'a str>,
    pub body: &'a [u8],
}

/// What a verified delivery was.
#[derive(Debug)]
pub enum InboundOutcome {
    /// A verification challenge to echo.
    Challenge(String),
    /// An event, handed to the pipeline.
    Event(TriggerEventOutcome),
}

/// Why a delivery was refused.
#[derive(Debug)]
pub enum InboundRejection {
    /// No live subscription for this callback: the sender should stop.
    Gone,
    /// Over the 256 KiB body cap.
    TooLarge,
    /// Signature or timestamp failed verification.
    Unauthorized(VerifyError),
    /// Verified but malformed.
    BadRequest(&'static str),
    /// Verified, but the run could not be started; the sender may retry.
    Failed(CommandError),
}

/// Owns MCP event trigger subscriptions: the outbound `events/*` calls, the
/// refresher, and the inbound callback.
pub struct McpEventTriggers {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    egress: Arc<dyn EgressService>,
    api_base_url: String,
    system_flags: crate::records::FeatureFlagPolicy,
}

impl McpEventTriggers {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        egress: Arc<dyn EgressService>,
        api_base_url: impl Into<String>,
        system_flags: crate::records::FeatureFlagPolicy,
    ) -> Self {
        Self {
            db,
            encryption,
            egress,
            api_base_url: api_base_url.into().trim_end_matches('/').to_string(),
            system_flags,
        }
    }

    /// The service as the server wires it, from the server's auth state.
    pub fn shared(
        db: &Arc<StorageBackend>,
        encryption: &Option<Arc<EncryptionService>>,
        host: &everruns_core::host::HostComposition,
        auth: &crate::auth::AuthState,
    ) -> Arc<Self> {
        Arc::new(Self::new(
            db.clone(),
            encryption.clone(),
            host.egress_service(),
            auth.config.base_url.clone(),
            auth.feature_flag_policy.clone(),
        ))
    }

    /// Run [`Self::refresh_due`] forever, every
    /// `MCP_EVENT_TRIGGER_REFRESH_INTERVAL_SECONDS` (default 60).
    pub fn spawn_refresher(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval = std::env::var("MCP_EVENT_TRIGGER_REFRESH_INTERVAL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|&seconds| seconds > 0)
            .map_or(DEFAULT_REFRESH_INTERVAL, Duration::from_secs);
        let service = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                service.refresh_due().await;
            }
        })
    }

    /// The URL the MCP server posts this trigger's events to.
    pub fn callback_url(&self, ingress_id: &str) -> String {
        format!("{}/v1/e/{ingress_id}/{CALLBACK_SEGMENT}", self.api_base_url)
    }

    fn encryption(&self) -> Result<&Arc<EncryptionService>, CommandError> {
        self.encryption.as_ref().ok_or_else(|| {
            CommandError::bad_request("MCP event triggers need secrets encryption configured")
        })
    }

    async fn org_enabled(&self, org_id: i64) -> bool {
        crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            org_id,
            &self.system_flags,
        )
        .await
        .is_ok_and(|flags| flags.mcp_events)
    }

    /// `events/subscribe` for one trigger and store the result. With
    /// `fresh_secret` (create, enable, config change) a new signing secret is
    /// generated; a refresh keeps the current one and passes back the cursor.
    pub async fn subscribe(
        &self,
        trigger: &AgentTriggerRow,
        fresh_secret: bool,
    ) -> Result<AgentTriggerMcpSubscriptionRow, CommandError> {
        let encryption = self.encryption()?;
        let config = parse_config(trigger)?;
        let ingress_id = trigger
            .ingress_id
            .as_deref()
            .ok_or_else(|| CommandError::bad_request("MCP event trigger has no ingress"))?;
        let agent = self
            .db
            .get_agent(trigger.org_id, trigger.agent_id)
            .await
            .map_err(classify_anyhow)?
            .filter(|agent| agent.status == "active")
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        let existing = self
            .db
            .get_agent_trigger_mcp_subscription(trigger.id)
            .await
            .map_err(classify_anyhow)?;
        let (secret, mut state) = match existing.filter(|_| !fresh_secret) {
            Some(row) => {
                let secret = encryption
                    .decrypt_to_string(&row.secret_encrypted)
                    .map_err(CommandError::internal)?;
                (secret, UpsertAgentTriggerMcpSubscription::from(row))
            }
            None => {
                let secret = standard_webhooks::generate_secret();
                let state = UpsertAgentTriggerMcpSubscription {
                    trigger_id: trigger.id,
                    org_id: trigger.org_id,
                    secret_encrypted: encryption
                        .encrypt_string(&secret)
                        .map_err(CommandError::internal)?,
                    remote_subscription_id: None,
                    refresh_before: None,
                    cursor: None,
                    status: MCP_SUBSCRIPTION_PENDING.to_string(),
                    last_error: None,
                };
                // Stored before the call: the server challenges the callback
                // while `events/subscribe` is in flight, and the challenge is
                // verified against this secret.
                self.db
                    .upsert_agent_trigger_mcp_subscription(state.clone())
                    .await
                    .map_err(classify_anyhow)?;
                (secret, state)
            }
        };

        let mut params = json!({
            "name": config.event,
            "arguments": config.arguments,
            "delivery": {
                "mode": "webhook",
                "url": self.callback_url(ingress_id),
                "secret": secret,
            },
        });
        if let Some(cursor) = &state.cursor {
            params["cursor"] = Value::String(cursor.clone());
        }
        let result = match self.remote(&agent, &config.server, trigger).await {
            Ok(remote) => {
                remote
                    .call(&*self.egress, "events/subscribe", &params)
                    .await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(result) => {
                state.remote_subscription_id = result
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|id| truncate(id, MAX_NAME_LEN));
                state.refresh_before = Some(parse_refresh_before(
                    result.get("refreshBefore"),
                    Utc::now(),
                ));
                if let Some(cursor) = result.get("cursor").and_then(Value::as_str) {
                    state.cursor = Some(truncate(cursor, MAX_CURSOR_LEN));
                }
                state.status = MCP_SUBSCRIPTION_ACTIVE.to_string();
                state.last_error = None;
                self.db
                    .upsert_agent_trigger_mcp_subscription(state)
                    .await
                    .map_err(classify_anyhow)
            }
            Err(error) => {
                if state.status != MCP_SUBSCRIPTION_PENDING {
                    state.status = MCP_SUBSCRIPTION_FAILED.to_string();
                    state.refresh_before = Some(Utc::now() + RETRY_BACKOFF);
                }
                state.last_error = Some(truncate(&error.message(), 512));
                if let Err(store_error) = self.db.upsert_agent_trigger_mcp_subscription(state).await
                {
                    tracing::warn!(error = %store_error, trigger_id = %trigger.id, "failed to record MCP subscribe failure");
                }
                Err(error)
            }
        }
    }

    /// `events/unsubscribe` (best effort) and drop the local subscription.
    pub async fn unsubscribe(&self, trigger: &AgentTriggerRow) {
        let subscription = self
            .db
            .get_agent_trigger_mcp_subscription(trigger.id)
            .await
            .ok()
            .flatten();
        if let (Some(subscription), Ok(config), Some(ingress_id)) = (
            subscription,
            parse_config(trigger),
            trigger.ingress_id.as_deref(),
        ) {
            let agent = self
                .db
                .get_agent(trigger.org_id, trigger.agent_id)
                .await
                .ok()
                .flatten();
            let params = json!({
                "id": subscription.remote_subscription_id,
                "name": config.event,
                "arguments": config.arguments,
                "delivery": { "mode": "webhook", "url": self.callback_url(ingress_id) },
            });
            let outcome = match agent {
                Some(agent) => match self.remote(&agent, &config.server, trigger).await {
                    Ok(remote) => remote
                        .call(&*self.egress, "events/unsubscribe", &params)
                        .await
                        .map(|_| ()),
                    Err(error) => Err(error),
                },
                None => Err(CommandError::not_found("Agent")),
            };
            if let Err(error) = outcome {
                // The callback answers 410 from now on, which ends the
                // subscription on the server's next delivery.
                tracing::info!(trigger_id = %trigger.id, error = %error, "MCP events/unsubscribe failed; relying on 410");
            }
        }
        if let Err(error) = self
            .db
            .delete_agent_trigger_mcp_subscription(trigger.id)
            .await
        {
            tracing::warn!(%error, trigger_id = %trigger.id, "failed to delete MCP event subscription");
        }
    }

    /// Re-subscribe every subscription due within the refresh margin. Returns
    /// how many were refreshed.
    pub async fn refresh_due(&self) -> usize {
        let due = match self
            .db
            .list_agent_trigger_mcp_subscriptions_due(Utc::now() + REFRESH_MARGIN, REFRESH_BATCH)
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(%error, "failed to list due MCP event subscriptions");
                return 0;
            }
        };
        let mut refreshed = 0;
        for subscription in due {
            let trigger = self
                .db
                .get_agent_trigger(subscription.org_id, subscription.trigger_id)
                .await
                .ok()
                .flatten()
                .filter(|row| {
                    row.status == "active"
                        && row.enabled
                        && row.trigger_type == AgentTriggerType::McpEvent.to_string()
                });
            let Some(trigger) = trigger else {
                let _ = self
                    .db
                    .delete_agent_trigger_mcp_subscription(subscription.trigger_id)
                    .await;
                continue;
            };
            if !self.org_enabled(trigger.org_id).await {
                continue;
            }
            match self.subscribe(&trigger, false).await {
                Ok(_) => refreshed += 1,
                Err(error) => {
                    tracing::warn!(trigger_id = %trigger.id, error = %error, "MCP event subscription refresh failed");
                }
            }
        }
        refreshed
    }

    /// Transport and credential for the agent's MCP server attachment, the
    /// same ones the agent's tool calls use.
    async fn remote(
        &self,
        agent: &AgentRow,
        server: &str,
        trigger: &AgentTriggerRow,
    ) -> Result<Remote, CommandError> {
        let attachment = find_attachment(&self.db, agent, server).await?;
        let mcp_servers = McpServerService::with_egress_service(
            self.db.clone(),
            self.encryption.clone(),
            self.egress.clone(),
        );
        let resolved = scoped_mcp::resolve_matched_scoped_mcp_server(
            &mcp_servers,
            agent.org_id,
            trigger.id.uuid(),
            Some(attachment),
        )
        .await
        .map_err(|error| CommandError::bad_request(error.to_string()))?
        .ok_or_else(|| CommandError::bad_request(format!("MCP server '{server}' not found")))?;

        let mut headers = resolved.headers;
        let has_authorization = |headers: &HashMap<String, String>| {
            headers
                .keys()
                .any(|key| key.eq_ignore_ascii_case("authorization"))
        };
        if let Some(api_key) = resolved.api_key
            && !has_authorization(&headers)
        {
            headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
        }
        if resolved.auth_mode == McpServerAuthMode::OAuth
            && !has_authorization(&headers)
            && let Some(provider) = resolved.oauth_provider_id.as_deref()
        {
            if resolved.acts_as != McpServerActsAs::Service {
                return Err(CommandError::bad_request(format!(
                    "MCP server '{server}' needs a service connection for an MCP event trigger"
                )));
            }
            let resolver = crate::storage::DbConnectionResolver::new(
                self.db.as_ref().clone(),
                self.encryption()?.as_ref().clone(),
                None,
                self.egress.clone(),
            );
            let token = resolver
                .agent_service_mcp_token(agent.org_id, agent.id, provider)
                .await
                .map_err(|error| CommandError::internal(anyhow::anyhow!(error.to_string())))?
                .ok_or_else(|| {
                    CommandError::bad_request(format!(
                        "Authorize the agent's connection to MCP server '{server}' first"
                    ))
                })?;
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
        Ok(Remote {
            url: resolved.url,
            headers,
        })
    }

    /// Verify and handle one delivery on a trigger's callback.
    pub async fn receive(
        &self,
        session_service: &SessionService,
        message_service: &MessageService,
        delivery: InboundDelivery<'_>,
        request_id: Option<String>,
    ) -> Result<InboundOutcome, InboundRejection> {
        if delivery.body.len() > standard_webhooks::MAX_BODY_BYTES {
            return Err(InboundRejection::TooLarge);
        }
        let failed = |error: anyhow::Error| InboundRejection::Failed(classify_anyhow(error));
        let trigger = self
            .db
            .get_agent_trigger_by_ingress_id_unscoped(delivery.ingress_id)
            .await
            .map_err(failed)?
            .filter(|row| row.enabled && row.trigger_type == AgentTriggerType::McpEvent.to_string())
            .ok_or(InboundRejection::Gone)?;
        let subscription = self
            .db
            .get_agent_trigger_mcp_subscription(trigger.id)
            .await
            .map_err(failed)?
            .ok_or(InboundRejection::Gone)?;
        let key = self
            .encryption
            .as_ref()
            .and_then(|encryption| {
                encryption
                    .decrypt_to_string(&subscription.secret_encrypted)
                    .ok()
            })
            .and_then(|secret| standard_webhooks::decode_secret(&secret).ok())
            .ok_or(InboundRejection::Gone)?;

        // THREAT[TM-TRIGGER-005]: the signature is the only gate, checked
        // before the body is parsed, with the timestamp window bounding replay.
        let webhook_id = standard_webhooks::verify(
            &key,
            SignedHeaders {
                id: delivery.webhook_id,
                timestamp: delivery.timestamp,
                signature: delivery.signature,
            },
            delivery.body,
            Utc::now().timestamp(),
        )
        .map_err(InboundRejection::Unauthorized)?;

        let body: Value = serde_json::from_slice(delivery.body)
            .map_err(|_| InboundRejection::BadRequest("body is not JSON"))?;
        if body.get("type").and_then(Value::as_str) == Some("verification") {
            let challenge = body
                .get("challenge")
                .and_then(Value::as_str)
                .filter(|challenge| !challenge.is_empty() && challenge.len() <= MAX_CHALLENGE_LEN)
                .ok_or(InboundRejection::BadRequest("invalid challenge"))?;
            return Ok(InboundOutcome::Challenge(challenge.to_string()));
        }

        // A superseded subscription (the trigger re-subscribed with other
        // arguments, say) is told to stop.
        if let (Some(expected), Some(presented)) = (
            subscription.remote_subscription_id.as_deref(),
            delivery.subscription_id,
        ) && expected != presented
        {
            return Err(InboundRejection::Gone);
        }
        if !self.org_enabled(trigger.org_id).await {
            return Err(InboundRejection::Gone);
        }
        let config = parse_config(&trigger).map_err(|_| InboundRejection::Gone)?;
        let name = body
            .get("name")
            .and_then(Value::as_str)
            .ok_or(InboundRejection::BadRequest("event name is missing"))?;
        if name != config.event {
            return Err(InboundRejection::BadRequest(
                "event is not the one this trigger subscribed to",
            ));
        }
        let agent = self
            .db
            .get_agent(trigger.org_id, trigger.agent_id)
            .await
            .map_err(failed)?
            .filter(|agent| agent.status == "active" && !agent.exposures_suspended)
            .ok_or(InboundRejection::Gone)?;

        if let Some(cursor) = body.get("cursor").and_then(Value::as_str) {
            let mut state = UpsertAgentTriggerMcpSubscription::from(subscription.clone());
            state.cursor = Some(truncate(cursor, MAX_CURSOR_LEN));
            if let Err(error) = self.db.upsert_agent_trigger_mcp_subscription(state).await {
                tracing::warn!(%error, trigger_id = %trigger.id, "failed to store MCP event cursor");
            }
        }

        let context = json!({
            "agent": { "id": agent.public_id, "name": agent.name },
            "trigger": { "id": trigger.id.to_string(), "type": "mcp_event" },
            "invocation": {
                "source": "mcp_event",
                "triggered_at": Utc::now().to_rfc3339(),
            },
            "mcp": {
                "server": config.server,
                "event": name,
                "event_id": body.get("eventId"),
                "timestamp": body.get("timestamp"),
                "subscription_id": subscription.remote_subscription_id,
                "delivery_id": webhook_id,
            },
            "payload": body.get("data").cloned().unwrap_or(Value::Null),
        });
        let subject = config
            .subject_template
            .as_deref()
            .map(|template| render_message_template(template, &context));
        let outcome = events::dispatch_trigger_event(
            &self.db,
            session_service,
            message_service,
            TriggerEventRoute {
                trigger: &trigger,
                agent: &agent,
                message_template: &config.message,
                session_mode: config.session_mode,
                filter: config.filter.as_ref(),
                session_source: crate::records::SessionSource::Webhook,
                webhook_compat: None,
            },
            TriggerEvent {
                source: "mcp_event",
                // THREAT[TM-TRIGGER-001]: `webhook-id` is the replay key; the
                // pipeline records a repeat as a duplicate.
                event_id: Some(webhook_id.to_string()),
                event_type: Some(name.to_string()),
                subject,
                context,
            },
            request_id,
        )
        .await
        .map_err(InboundRejection::Failed)?;
        Ok(InboundOutcome::Event(outcome))
    }
}

/// A resolved MCP endpoint with its credential baked into the headers.
struct Remote {
    url: String,
    headers: HashMap<String, String>,
}

impl Remote {
    async fn call(
        &self,
        egress: &dyn EgressService,
        method: &str,
        params: &Value,
    ) -> Result<Value, CommandError> {
        everruns_core::mcp::http_request(egress, &self.url, &self.headers, None, method, params)
            .await
            .map_err(|error| {
                if let Some(rpc) = error.downcast_ref::<everruns_core::mcp::McpRpcError>() {
                    return CommandError::unprocessable(format!(
                        "MCP server rejected {method}: {}",
                        rpc.message
                    ));
                }
                CommandError::unavailable(format!("MCP server {method} failed: {error}"))
            })
    }
}

fn parse_config(trigger: &AgentTriggerRow) -> Result<McpEventTriggerConfig, CommandError> {
    serde_json::from_value(trigger.config.clone())
        .map_err(|_| CommandError::bad_request("Invalid MCP event trigger configuration"))
}

/// `refreshBefore` as RFC 3339 or epoch milliseconds; a server that sends
/// neither gets [`DEFAULT_LIFETIME`].
fn parse_refresh_before(value: Option<&Value>, now: DateTime<Utc>) -> DateTime<Utc> {
    let parsed = match value {
        Some(Value::String(text)) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|at| at.with_timezone(&Utc)),
        Some(Value::Number(number)) => number
            .as_i64()
            .and_then(DateTime::<Utc>::from_timestamp_millis),
        _ => None,
    };
    parsed.unwrap_or(now + DEFAULT_LIFETIME)
}

fn truncate(value: &str, max: usize) -> String {
    let mut end = value.len().min(max);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_default_to_an_object_and_reject_other_shapes() {
        assert_eq!(normalize_arguments(None).unwrap(), json!({}));
        assert_eq!(normalize_arguments(Some(Value::Null)).unwrap(), json!({}));
        assert_eq!(
            normalize_arguments(Some(json!({"team": "eng"}))).unwrap(),
            json!({"team": "eng"})
        );
        assert!(normalize_arguments(Some(json!(["team"]))).is_err());
        assert!(normalize_arguments(Some(json!("team"))).is_err());
        let huge = json!({ "x": "y".repeat(MAX_ARGUMENTS_BYTES) });
        assert!(normalize_arguments(Some(huge)).is_err());
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(require_name("mcp_event", Some(" a.b ")).unwrap(), "a.b");
        assert!(require_name("mcp_event", None).is_err());
        assert!(require_name("mcp_event", Some("  ")).is_err());
        assert!(require_name("mcp_event", Some(&"x".repeat(MAX_NAME_LEN + 1))).is_err());
    }

    #[test]
    fn refresh_before_accepts_rfc3339_and_millis() {
        let now = Utc::now();
        let at = parse_refresh_before(Some(&json!("2030-01-01T00:00:00Z")), now);
        assert_eq!(at.to_rfc3339(), "2030-01-01T00:00:00+00:00");
        let at = parse_refresh_before(Some(&json!(1_893_456_000_000_i64)), now);
        assert_eq!(at.timestamp(), 1_893_456_000);
        assert_eq!(parse_refresh_before(None, now), now + DEFAULT_LIFETIME);
        assert_eq!(
            parse_refresh_before(Some(&json!("soon")), now),
            now + DEFAULT_LIFETIME
        );
    }
}
