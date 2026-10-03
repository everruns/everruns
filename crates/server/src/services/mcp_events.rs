//! Outbound MCP Events (EVE-1121): webhook subscriptions for session
//! completion, failure, and input required, delivered to MCP clients such as
//! ChatGPT. See `knowledge/integrations/mcp-events.md` for the pinned spec
//! revision and the design.
//!
//! Decision: webhook delivery only. The draft spec also has in-band delivery on
//! an open stream, but `/mcp` is stateless and ChatGPT asks for webhooks.
//!
//! Decision: payloads carry identifiers and state, never transcript text. The
//! subscriber reads content through `session_get_status` under its own token,
//! so a webhook cannot become a side channel for model output (payload
//! injection) and carries nothing a stale subscription should not see.
//!
//! Decision: access is re-checked at every delivery, not only at subscribe
//! time: the subscriber must still be a member of the org, still hold session
//! view, and the org must still have the `mcp_events` flag. A subscriber who
//! left the org has the subscription deleted. Platform Chat sessions are never
//! announced, because they are private to their owner (TM-AGENT-017).

use std::sync::Arc;
use std::time::Duration;

use crate::records::FeatureFlags;
use anyhow::Context as _;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_core::events::{TOOL_CALL_REQUESTED, TOOL_COMPLETED, TURN_COMPLETED, TURN_FAILED};
use everruns_core::{
    Caller, EgressRequest, EgressRequestKind, EgressService, Event, EventListener, OrgRole,
    PermissionResolver,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::storage::encryption::EncryptionService;
use crate::storage::{McpEventSubscriptionRow, StorageBackend, UpsertMcpEventSubscription};

use super::standard_webhooks::{self, MAX_BODY_BYTES, sign};

pub const SESSION_COMPLETED: &str = "session.completed";
pub const SESSION_FAILED: &str = "session.failed";
pub const SESSION_INPUT_REQUIRED: &str = "session.input_required";
pub const EVENT_NAMES: [&str; 3] = [SESSION_COMPLETED, SESSION_FAILED, SESSION_INPUT_REQUIRED];

/// JSON-RPC error for a callback that failed verification (draft spec).
pub const CALLBACK_VERIFICATION_FAILED: i64 = -32015;
const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MIN_TTL: Duration = Duration::from_secs(60);
const MAX_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const REQUEST_TIMEOUT_MS: u64 = 10_000;
const ASK_USER_TOOL: &str = "ask_user";

/// An `events/*` failure, shaped for a JSON-RPC error response.
#[derive(Debug, Clone, PartialEq)]
pub struct EventsError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl EventsError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }

    fn verification(reason: &str) -> Self {
        Self {
            code: CALLBACK_VERIFICATION_FAILED,
            message: "Callback verification failed".to_string(),
            data: Some(json!({ "reason": reason })),
        }
    }
}

/// Who is subscribing: the MCP token's user in the org the request resolved to.
#[derive(Debug, Clone)]
pub struct Subscriber {
    pub org_id: i64,
    pub user_id: Uuid,
}

/// Parsed `events/subscribe` or `events/unsubscribe` target.
#[derive(Debug, Clone)]
pub struct SubscriptionTarget {
    pub name: String,
    pub arguments: Value,
    pub url: String,
}

impl SubscriptionTarget {
    /// Parse `{name, arguments, delivery: {mode: "webhook", url}}`.
    pub fn from_params(params: &Value) -> Result<Self, EventsError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| EventsError::invalid("name is required"))?;
        if !EVENT_NAMES.contains(&name) {
            return Err(EventsError::invalid(format!("Unknown event: {name}")));
        }
        let arguments = validate_arguments(params.get("arguments"))?;
        let delivery = params
            .get("delivery")
            .ok_or_else(|| EventsError::invalid("delivery is required"))?;
        if delivery.get("mode").and_then(Value::as_str) != Some("webhook") {
            return Err(EventsError::invalid("Only webhook delivery is supported"));
        }
        let url = delivery
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| EventsError::invalid("delivery.url is required"))?;
        Ok(Self {
            name: name.to_string(),
            arguments,
            url: url.to_string(),
        })
    }
}

/// Result of a successful `events/subscribe`.
#[derive(Debug, Clone)]
pub struct Subscribed {
    pub id: String,
    pub refresh_before: DateTime<Utc>,
}

/// Owns subscriptions and delivery. Shared by the `/mcp` handlers and the
/// event listener.
pub struct McpEventsService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    egress: Arc<dyn EgressService>,
    system_flags: FeatureFlags,
    ui_base: Option<String>,
    retry_delays: Vec<Duration>,
    permission_resolver: Arc<dyn PermissionResolver>,
}

impl McpEventsService {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        egress: Arc<dyn EgressService>,
        system_flags: FeatureFlags,
    ) -> Self {
        Self {
            db,
            encryption,
            egress,
            system_flags,
            ui_base: None,
            retry_delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(10),
                Duration::from_secs(60),
            ],
            permission_resolver: Arc::new(everruns_core::DefaultPermissionResolver),
        }
    }

    /// The service as the server wires it: deployment flags, payload links to
    /// the UI, and the permission resolver for delivery-time access checks, all
    /// taken from the server's auth state. Inert until an org opts in.
    pub fn shared(
        db: &Arc<StorageBackend>,
        encryption: &Option<Arc<EncryptionService>>,
        host: &everruns_core::host::HostComposition,
        auth: &crate::auth::AuthState,
    ) -> Arc<Self> {
        let flags = auth.system_feature_flags.clone();
        Arc::new(
            Self::new(db.clone(), encryption.clone(), host.egress_service(), flags)
                .with_ui_base(&auth.config.frontend_url)
                .with_permission_resolver(auth.permission_resolver.clone()),
        )
    }

    /// The event listener that feeds this service.
    pub fn listener(self: &Arc<Self>) -> Arc<dyn EventListener> {
        Arc::new(McpEventsListener::new(self.clone()))
    }

    /// UI root used for the `link` field in payloads.
    pub fn with_ui_base(mut self, ui_base: impl Into<String>) -> Self {
        self.ui_base = Some(ui_base.into().trim_end_matches('/').to_string());
        self
    }

    /// Waits between delivery attempts; one retry per entry.
    pub fn with_retry_delays(mut self, retry_delays: Vec<Duration>) -> Self {
        self.retry_delays = retry_delays;
        self
    }

    pub fn with_permission_resolver(mut self, resolver: Arc<dyn PermissionResolver>) -> Self {
        self.permission_resolver = resolver;
        self
    }

    /// `events/list`: the catalog. Static, so the same for every caller.
    pub fn event_definitions() -> Value {
        let filter = json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Only sessions of this agent." },
                "session_id": { "type": "string", "description": "Only this session." }
            },
            "additionalProperties": false
        });
        let describe = |name: &str, description: &str, extra: Value| {
            let mut properties = json!({
                "session_id": { "type": "string" },
                "agent_id": { "type": ["string", "null"] },
                "title": { "type": ["string", "null"] },
                "link": { "type": ["string", "null"], "description": "The session in Everruns." }
            });
            if let (Some(map), Some(more)) = (properties.as_object_mut(), extra.as_object()) {
                map.extend(more.clone());
            }
            json!({
                "name": name,
                "description": description,
                "delivery": ["webhook"],
                "inputSchema": filter,
                "payloadSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": ["session_id"]
                }
            })
        };
        json!([
            describe(
                SESSION_COMPLETED,
                "An agent finished its turn and the session is idle. Read the reply with \
                 session_get_status.",
                json!({}),
            ),
            describe(
                SESSION_FAILED,
                "An agent's turn failed.",
                json!({ "error_code": { "type": ["string", "null"] } }),
            ),
            describe(
                SESSION_INPUT_REQUIRED,
                "An agent is waiting on the user: a question to answer or an action to \
                 approve.",
                json!({
                    "kind": { "type": "string", "enum": ["question", "approval"] },
                    "tool_call_id": { "type": ["string", "null"] }
                }),
            ),
        ])
    }

    /// `events/subscribe`: verify the callback, then create or refresh the
    /// subscription. Idempotent on (subscriber, url, name, arguments).
    pub async fn subscribe(
        &self,
        subscriber: &Subscriber,
        target: &SubscriptionTarget,
        secret: &str,
        ttl_ms: Option<u64>,
    ) -> Result<Subscribed, EventsError> {
        let key = decode_secret(secret)?;
        let encryption = self.encryption.as_ref().ok_or_else(|| {
            EventsError::internal("MCP Events need secrets encryption configured")
        })?;
        let id = subscription_id(subscriber, target);
        self.verify_callback(&target.url, &id, &key).await?;

        let ttl = ttl_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_TTL)
            .clamp(MIN_TTL, MAX_TTL);
        let expires_at = Utc::now()
            + chrono::Duration::from_std(ttl).map_err(|e| EventsError::internal(e.to_string()))?;
        let secret_encrypted = encryption
            .encrypt_string(secret)
            .map_err(|e| EventsError::internal(e.to_string()))?;
        self.db
            .upsert_mcp_event_subscription(UpsertMcpEventSubscription {
                subscription_key: id.clone(),
                org_id: subscriber.org_id,
                user_id: subscriber.user_id,
                event_name: target.name.clone(),
                arguments: target.arguments.clone(),
                callback_url: target.url.clone(),
                secret_encrypted,
                expires_at,
            })
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to store MCP event subscription");
                EventsError::internal("Failed to store the subscription")
            })?;
        Ok(Subscribed {
            id,
            refresh_before: expires_at,
        })
    }

    /// `events/unsubscribe`: remove the subscriber's own subscription, if any.
    pub async fn unsubscribe(
        &self,
        subscriber: &Subscriber,
        target: &SubscriptionTarget,
    ) -> Result<(), EventsError> {
        let id = subscription_id(subscriber, target);
        self.db
            .delete_mcp_event_subscription(&id)
            .await
            .map_err(|e| EventsError::internal(e.to_string()))?;
        Ok(())
    }

    /// Challenge the callback before any event goes to it: the receiver has to
    /// echo the challenge in a signed round trip, so a subscription cannot aim
    /// Everruns at a URL whose owner did not ask for it.
    async fn verify_callback(&self, url: &str, id: &str, key: &[u8]) -> Result<(), EventsError> {
        let challenge = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let body = serde_json::to_vec(&json!({ "type": "verification", "challenge": challenge }))
            .map_err(|e| EventsError::internal(e.to_string()))?;
        let message_id = format!("msg_verify_{}", Uuid::now_v7().simple());
        let response =
            self.post(url, id, &message_id, key, body)
                .await
                .map_err(|error| match error {
                    PostError::Blocked(reason) => EventsError::invalid(reason),
                    PostError::Transport(detail) if detail.to_lowercase().contains("timed out") => {
                        EventsError::verification("timeout")
                    }
                    PostError::Transport(_) => EventsError::verification("challenge_failed"),
                })?;
        let echoed = serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|value| value.get("challenge")?.as_str().map(str::to_string));
        let matches = echoed.is_some_and(|echoed| constant_time_eq(&echoed, &challenge));
        if !(200..300).contains(&response.status) || !matches {
            return Err(EventsError::verification("challenge_failed"));
        }
        Ok(())
    }

    /// Deliver one Everruns event to every matching subscription. Returns how
    /// many deliveries were accepted. Awaits retries, so the listener runs it
    /// in the background.
    pub async fn dispatch(&self, event: &Event) -> usize {
        let Some((name, data)) = classify(event) else {
            return 0;
        };
        match self.dispatch_classified(event, name, data).await {
            Ok(delivered) => delivered,
            Err(error) => {
                tracing::warn!(error = %error, session_id = %event.session_id, "MCP event dispatch failed");
                0
            }
        }
    }

    async fn dispatch_classified(
        &self,
        event: &Event,
        name: &'static str,
        extra: Value,
    ) -> anyhow::Result<usize> {
        let Some(session) = self.db.get_session_unscoped(event.session_id).await? else {
            return Ok(0);
        };
        let subscriptions = self
            .db
            .list_active_mcp_event_subscriptions(session.org_id, name, Utc::now())
            .await?;
        if subscriptions.is_empty() {
            return Ok(0);
        }
        let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &self.db,
            session.org_id,
            &self.system_flags,
        )
        .await?;
        if !flags.mcp_events || self.is_platform_chat(&session).await? {
            return Ok(0);
        }

        let session_id = session.id.to_string();
        let agent_id = session.agent_id.map(|id| id.to_string());
        let mut data = json!({
            "session_id": session_id,
            "agent_id": agent_id,
            "title": session.title,
            "link": self.ui_base.as_ref().map(|base| format!("{base}/sessions/{session_id}/chat")),
        });
        if let (Some(map), Some(more)) = (data.as_object_mut(), extra.as_object()) {
            map.extend(more.clone());
        }

        let mut delivered = 0;
        for subscription in subscriptions {
            if !filter_matches(&subscription.arguments, &session_id, agent_id.as_deref()) {
                continue;
            }
            if !self.subscriber_may_view(&subscription).await? {
                continue;
            }
            let body = json!({
                "eventId": event.id.to_string(),
                "name": name,
                "timestamp": event.ts.to_rfc3339(),
                "data": data,
                "cursor": Value::Null,
            });
            if self
                .deliver(&subscription, &event.id.to_string(), &body)
                .await
            {
                delivered += 1;
            }
        }
        Ok(delivered)
    }

    async fn is_platform_chat(&self, session: &crate::storage::SessionRow) -> anyhow::Result<bool> {
        let Some(harness_id) = session.harness_id else {
            return Ok(false);
        };
        Ok(self
            .db
            .get_harness(session.org_id, harness_id)
            .await?
            .is_some_and(|harness| harness.is_built_in && harness.name == "platform-chat"))
    }

    async fn subscriber_may_view(
        &self,
        subscription: &McpEventSubscriptionRow,
    ) -> anyhow::Result<bool> {
        let Some(member) = self
            .db
            .get_organization_member(subscription.org_id, subscription.user_id)
            .await?
        else {
            // The subscriber left the org: nothing they subscribed to is
            // theirs to hear about any more.
            self.db
                .delete_mcp_event_subscription(&subscription.subscription_key)
                .await?;
            return Ok(false);
        };
        let caller = Caller {
            org_id: subscription.org_id,
            org_public_id: String::new(),
            user_id: Some(subscription.user_id),
            role: member.role.parse::<OrgRole>().unwrap_or(OrgRole::Member),
            is_platform_user: false,
            is_internal: false,
        };
        let allowed = crate::domains::sessions::SESSION_VIEW
            .evaluate_with(self.permission_resolver.as_ref(), &caller);
        Ok(allowed.is_ok())
    }

    /// Sign and POST one event, retrying on failure. `410 Gone` ends the
    /// subscription; `413` is not retried, since the same body will not fit
    /// next time either.
    async fn deliver(
        &self,
        subscription: &McpEventSubscriptionRow,
        event_id: &str,
        body: &Value,
    ) -> bool {
        let Some(key) = self.subscription_key_bytes(subscription) else {
            return false;
        };
        let Ok(bytes) = serde_json::to_vec(body) else {
            return false;
        };
        if bytes.len() > MAX_BODY_BYTES {
            tracing::warn!(subscription = %subscription.subscription_key, "MCP event body over the size cap");
            return false;
        }
        let message_id = format!("msg_{}", event_id.trim_start_matches("event_"));
        let mut delays = self.retry_delays.iter();
        loop {
            // Each attempt is signed fresh: a new timestamp, so a receiver's
            // replay window never rejects a legitimate retry.
            let outcome = self
                .post(
                    &subscription.callback_url,
                    &subscription.subscription_key,
                    &message_id,
                    &key,
                    bytes.clone(),
                )
                .await;
            match outcome {
                Ok(response) if (200..300).contains(&response.status) => return true,
                Ok(response) if response.status == 410 => {
                    if let Err(error) = self
                        .db
                        .delete_mcp_event_subscription(&subscription.subscription_key)
                        .await
                    {
                        tracing::warn!(error = %error, "Failed to drop a gone MCP event subscription");
                    }
                    return false;
                }
                Ok(response) if response.status == 413 => return false,
                Err(PostError::Blocked(reason)) => {
                    tracing::warn!(reason = %reason, "MCP event callback blocked");
                    return false;
                }
                Ok(_) | Err(PostError::Transport(_)) => {}
            }
            let Some(delay) = delays.next() else {
                tracing::warn!(subscription = %subscription.subscription_key, "MCP event delivery gave up");
                return false;
            };
            tokio::time::sleep(*delay).await;
        }
    }

    fn subscription_key_bytes(&self, subscription: &McpEventSubscriptionRow) -> Option<Vec<u8>> {
        let secret = self
            .encryption
            .as_ref()?
            .decrypt_to_string(&subscription.secret_encrypted)
            .inspect_err(
                |error| tracing::warn!(error = %error, "Failed to decrypt MCP event secret"),
            )
            .ok()?;
        decode_secret(&secret).ok()
    }

    async fn post(
        &self,
        url: &str,
        subscription_id: &str,
        message_id: &str,
        key: &[u8],
        body: Vec<u8>,
    ) -> Result<everruns_core::EgressResponse, PostError> {
        // THREAT[TM-MCP-010]: callbacks are client-chosen URLs. HTTPS only,
        // resolved and pinned so a name cannot rebind to a private address
        // between the check and the connect (TM-TOOL-018); the egress client
        // follows no redirects.
        if !url.starts_with("https://") {
            return Err(PostError::Blocked(
                "callback URL must use https".to_string(),
            ));
        }
        let (parsed, pinned) = everruns_contracts::url_validation::validate_url_dns_pinned(url)
            .await
            .map_err(|e| PostError::Blocked(format!("callback URL is not allowed: {e}")))?;
        let timestamp = Utc::now().timestamp().to_string();
        let signature = sign(key, message_id, &timestamp, &body);
        let mut request = EgressRequest::new("POST", url, EgressRequestKind::Mcp)
            .header("content-type", "application/json")
            .header("webhook-id", message_id)
            .header("webhook-timestamp", timestamp)
            .header("webhook-signature", signature)
            .header("x-mcp-subscription-id", subscription_id)
            .body(body)
            .timeout_ms(REQUEST_TIMEOUT_MS);
        request = if pinned.is_empty() {
            request.require_dns_pinning()
        } else {
            request.pinned_addrs(parsed.host_str().unwrap_or_default().to_string(), pinned)
        };
        self.egress
            .send(request)
            .await
            .map_err(|e| PostError::Transport(e.to_string()))
    }
}

enum PostError {
    Blocked(String),
    Transport(String),
}

/// Runs delivery off the event path: listeners must not block event storage.
pub struct McpEventsListener {
    service: Arc<McpEventsService>,
}

impl McpEventsListener {
    pub fn new(service: Arc<McpEventsService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EventListener for McpEventsListener {
    async fn on_event(&self, event: &Event) {
        if classify(event).is_none() {
            return;
        }
        let service = self.service.clone();
        let event = event.clone();
        tokio::spawn(async move {
            service.dispatch(&event).await;
        });
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            TURN_COMPLETED,
            TURN_FAILED,
            TOOL_CALL_REQUESTED,
            TOOL_COMPLETED,
        ])
    }

    fn name(&self) -> &'static str {
        "McpEventsListener"
    }
}

/// Map an Everruns event to the MCP event it announces, with the fields only
/// that event carries.
fn classify(event: &Event) -> Option<(&'static str, Value)> {
    let data = serde_json::to_value(&event.data).ok()?;
    match event.event_type.as_str() {
        TURN_COMPLETED => Some((SESSION_COMPLETED, json!({}))),
        TURN_FAILED => Some((
            SESSION_FAILED,
            json!({ "error_code": data.get("error_code").cloned().unwrap_or(Value::Null) }),
        )),
        TOOL_CALL_REQUESTED => {
            let call = data
                .get("tool_calls")?
                .as_array()?
                .iter()
                .find(|call| call.get("name").and_then(Value::as_str) == Some(ASK_USER_TOOL))?;
            Some((
                SESSION_INPUT_REQUIRED,
                json!({ "kind": "question", "tool_call_id": call.get("id").cloned() }),
            ))
        }
        TOOL_COMPLETED => {
            crate::slack_approvals::extract_approval_request(&data)?;
            Some((
                SESSION_INPUT_REQUIRED,
                json!({ "kind": "approval", "tool_call_id": data.get("tool_call_id").cloned() }),
            ))
        }
        _ => None,
    }
}

fn validate_arguments(arguments: Option<&Value>) -> Result<Value, EventsError> {
    let Some(arguments) = arguments.filter(|value| !value.is_null()) else {
        return Ok(json!({}));
    };
    let map = arguments
        .as_object()
        .ok_or_else(|| EventsError::invalid("arguments must be an object"))?;
    for (key, value) in map {
        if key != "agent_id" && key != "session_id" {
            return Err(EventsError::invalid(format!("Unknown argument: {key}")));
        }
        if !value.is_string() {
            return Err(EventsError::invalid(format!("{key} must be a string")));
        }
    }
    Ok(arguments.clone())
}

fn filter_matches(arguments: &Value, session_id: &str, agent_id: Option<&str>) -> bool {
    let wanted = |key: &str| arguments.get(key).and_then(Value::as_str);
    wanted("session_id").is_none_or(|wanted| wanted == session_id)
        && wanted("agent_id").is_none_or(|wanted| Some(wanted) == agent_id)
}

fn decode_secret(secret: &str) -> Result<Vec<u8>, EventsError> {
    standard_webhooks::decode_secret(secret).map_err(EventsError::invalid)
}

/// Stable id for (subscriber, url, name, canonical arguments), so a repeated
/// subscribe refreshes rather than duplicates.
fn subscription_id(subscriber: &Subscriber, target: &SubscriptionTarget) -> String {
    let mut hasher = Sha256::new();
    for part in [
        subscriber.org_id.to_string(),
        subscriber.user_id.to_string(),
        target.url.clone(),
        target.name.clone(),
        canonical_json(&target.arguments),
    ] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    format!("sub_{}", &hex::encode(hasher.finalize())[..32])
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        Value::String(key.clone()),
                        canonical_json(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        other => other.to_string(),
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    crate::security::constant_time_eq(a.as_bytes(), b.as_bytes())
}

/// Verify a Standard Webhooks signature. Public for tests and receivers built
/// on this crate.
pub fn verify_signature(
    secret: &str,
    message_id: &str,
    timestamp: &str,
    body: &[u8],
    header: &str,
) -> anyhow::Result<bool> {
    let key = decode_secret(secret)
        .map_err(|e| anyhow::anyhow!(e.message))
        .context("secret")?;
    Ok(standard_webhooks::signature_matches(
        &key, message_id, timestamp, body, header,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_id_ignores_argument_order_but_not_the_subscriber() {
        let subscriber = Subscriber {
            org_id: 1,
            user_id: Uuid::nil(),
        };
        let target = |arguments: Value| SubscriptionTarget {
            name: SESSION_COMPLETED.to_string(),
            arguments,
            url: "https://example.com/hook".to_string(),
        };
        let a = subscription_id(
            &subscriber,
            &target(json!({"agent_id": "a", "session_id": "s"})),
        );
        let b = subscription_id(
            &subscriber,
            &target(json!({"session_id": "s", "agent_id": "a"})),
        );
        assert_eq!(a, b);
        let other = Subscriber {
            org_id: 2,
            user_id: Uuid::nil(),
        };
        assert_ne!(
            a,
            subscription_id(&other, &target(json!({"agent_id": "a", "session_id": "s"})))
        );
    }

    #[test]
    fn target_rejects_unknown_events_arguments_and_modes() {
        let ok = json!({ "name": SESSION_FAILED, "delivery": { "mode": "webhook", "url": "https://x.example" } });
        assert!(SubscriptionTarget::from_params(&ok).is_ok());
        let mut bad = ok.clone();
        bad["name"] = json!("session.deleted");
        assert!(SubscriptionTarget::from_params(&bad).is_err());
        let mut bad = ok.clone();
        bad["arguments"] = json!({ "org_id": "o" });
        assert!(SubscriptionTarget::from_params(&bad).is_err());
        let mut bad = ok;
        bad["delivery"]["mode"] = json!("stream");
        assert!(SubscriptionTarget::from_params(&bad).is_err());
    }

    #[test]
    fn filters_match_on_session_and_agent() {
        assert!(filter_matches(&json!({}), "s1", None));
        assert!(filter_matches(
            &json!({"session_id": "s1"}),
            "s1",
            Some("a1")
        ));
        assert!(!filter_matches(
            &json!({"session_id": "s2"}),
            "s1",
            Some("a1")
        ));
        assert!(filter_matches(&json!({"agent_id": "a1"}), "s1", Some("a1")));
        assert!(!filter_matches(&json!({"agent_id": "a1"}), "s1", None));
    }

    #[test]
    fn catalog_lists_the_three_session_events() {
        let names: Vec<String> = McpEventsService::event_definitions()
            .as_array()
            .map(|events| {
                events
                    .iter()
                    .filter_map(|event| event["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(names, EVENT_NAMES);
    }
}
