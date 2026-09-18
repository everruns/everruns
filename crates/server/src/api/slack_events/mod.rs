use axum::{
    Router,
    routing::{get, post},
};
use everruns_worker::AgentRunner;
use hmac::Hmac;
use moka::sync::Cache;
use sha2::Sha256;
use std::sync::Arc;

use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::services::EventService;
use crate::slack_delivery::SlackDeliveryDispatcher;
use crate::storage::StorageBackend;

// Split out of one 5500-line file; the module's public surface is unchanged.
mod api;
mod content;
mod events;
mod manifest;
mod thread;
mod wire;

pub(crate) use api::*;
pub(crate) use content::*;
pub(crate) use events::*;
pub(crate) use manifest::*;
pub(crate) use thread::*;
pub(crate) use wire::*;

type HmacSha256 = Hmac<Sha256>;

/// Resolved Slack user display name.
/// `Some(name)` = resolved, `None` = resolution failed (don't retry).
type SlackUserCache = Cache<String, Option<String>>;

/// Bound for in-memory Slack users.info cache to prevent unbounded growth.
const SLACK_USER_CACHE_MAX_ENTRIES: u64 = 10_000;

fn new_slack_user_cache() -> SlackUserCache {
    Cache::builder()
        .max_capacity(SLACK_USER_CACHE_MAX_ENTRIES)
        .build()
}

/// App-scoped Slack state (no auth required).
#[derive(Clone)]
pub struct SlackState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<crate::storage::EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    /// Cache of Slack user ID → display name. Shared across requests.
    user_name_cache: SlackUserCache,
    /// Event-driven Slack delivery dispatcher (None in DEV_MODE without PostgreSQL).
    pub delivery_dispatcher: Option<Arc<SlackDeliveryDispatcher>>,
    /// Backend origin including the API prefix (e.g. `https://app.example.com/api`).
    /// The generated manifest needs it to name this server's own webhook URL.
    pub api_base_url: String,
}

impl SlackState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::EncryptionService>>,
        runner: Arc<dyn AgentRunner>,
        delivery_dispatcher: Option<Arc<SlackDeliveryDispatcher>>,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
        api_base_url: String,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                notifications_enabled,
                event_delivery.clone(),
            )),
            event_service: Arc::new(EventService::new(db.clone(), event_delivery)),
            encryption,
            db,
            user_name_cache: new_slack_user_cache(),
            delivery_dispatcher,
            api_base_url,
        }
    }
}

/// Create Slack webhook routes (no auth middleware).
pub fn routes(state: SlackState) -> Router {
    Router::new()
        .route(
            "/v1/apps/{app_id}/slack/events",
            post(handle_slack_event_legacy),
        )
        .route(
            "/v1/apps/{app_id}/slack/manifest",
            get(handle_slack_manifest_legacy),
        )
        .route(
            "/v1/e/{channel_id}/slack/events",
            post(handle_slack_event_endpoint),
        )
        .route(
            "/v1/e/{channel_id}/slack/manifest",
            get(handle_slack_manifest_endpoint),
        )
        .with_state(state)
}

pub(crate) enum SlackTarget {
    LegacyApp(String),
    Endpoint(String),
}

/// Extract text content from an output.message.completed event's data.
/// Delegates to the shared implementation in `slack_delivery`.
#[cfg(test)]
fn extract_response_text(data: &serde_json::Value) -> Option<String> {
    crate::slack_delivery::extract_response_text(data)
}

#[cfg(test)]
mod tests_manifest_build;
#[cfg(test)]
mod tests_routing;
#[cfg(test)]
mod tests_signature;
#[cfg(test)]
mod tests_support;
