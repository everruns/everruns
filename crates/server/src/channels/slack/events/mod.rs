use axum::{
    Router,
    routing::{get, post},
};
use everruns_core::host::TurnBackend;
use hmac::Hmac;
use moka::sync::Cache;
use sha2::Sha256;
use std::sync::Arc;

use crate::channels::host::{ServerChannelResolver, ServerChannels};
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::services::EventService;
use crate::storage::StorageBackend;

// Split out of one 5500-line file; the module's public surface is unchanged.
mod api;
mod content;
mod inbound;
mod interactivity;
mod manifest;
mod org_decisions;
mod response_policy;
mod thread;
mod wire;

pub(crate) use api::*;
pub(crate) use content::*;
pub(crate) use inbound::*;
pub(crate) use interactivity::*;
pub(crate) use manifest::*;
pub use org_decisions::SlackOrgDecisions;
pub(crate) use response_policy::should_process_message;
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
    pub(crate) user_name_cache: SlackUserCache,
    /// The channel host Slack runs on. The app always attaches one
    /// ([`with_channel_host`](Self::with_channel_host)); `None` only in tests
    /// that never reach intake.
    pub channels: Option<Arc<ServerChannels>>,
    /// Posts replies, streams and status to Slack. Tests point it at a mock.
    pub delivery_adapter: Arc<dyn everruns_core::channel::ChannelDeliveryAdapter>,
    /// UI base for the session link a turn's end notice carries; empty for
    /// none.
    pub frontend_url: String,
    /// Backend origin including the API prefix (e.g. `https://app.example.com/api`).
    /// The generated manifest needs it to name this server's own webhook URL.
    pub api_base_url: String,
    pub decisions: Arc<dyn everruns_core::DecisionsService>,
    /// What an org with `system_decisions: organization` decides with instead.
    pub org_decisions: Option<SlackOrgDecisions>,
}

impl SlackState {
    /// The UI link to a session, when the UI base is configured.
    pub(crate) fn session_link(
        &self,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Option<String> {
        let base = self.frontend_url.trim_end_matches('/');
        (!base.is_empty()).then(|| format!("{base}/sessions/{session_id}/chat"))
    }

    /// Inject the runtime's deployment decisions service, and the host
    /// services an org's own decision model runs on.
    pub fn with_decisions(
        mut self,
        host: &everruns_core::host::HostComposition,
        provider_resolver: &Arc<crate::services::ProviderResolverService>,
        budget_service: &Arc<crate::domains::budgets::BudgetService>,
    ) -> Self {
        self.decisions = host.decisions();
        self.org_decisions = Some(SlackOrgDecisions {
            provider_resolver: provider_resolver.clone(),
            budget_service: budget_service.clone(),
            egress: host.egress_service(),
        });
        self
    }

    /// Link turns' end notices to sessions under this UI base.
    pub fn with_frontend_url(mut self, frontend_url: impl Into<String>) -> Self {
        self.frontend_url = frontend_url.into();
        self
    }

    /// Attach the channel host Slack intake and approvals run on. Call last:
    /// the host's resolver keeps the state as it is now, without the host,
    /// which Slack drivers never need.
    pub fn with_channel_host(mut self) -> Self {
        let resolver = ServerChannelResolver::new(self.db.clone(), self.encryption.clone())
            .with_slack(self.clone());
        self.channels = Some(Arc::new(ServerChannels::with_resolver(
            resolver,
            self.db.clone(),
            self.session_service.clone(),
            self.message_service.clone(),
        )));
        self
    }

    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::EncryptionService>>,
        runner: Arc<dyn TurnBackend>,
        event_delivery: crate::live_updates::event_delivery::EventDelivery,
        api_base_url: String,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                event_delivery.clone(),
            )),
            event_service: Arc::new(EventService::new(db.clone(), event_delivery)),
            encryption,
            db,
            user_name_cache: new_slack_user_cache(),
            channels: None,
            delivery_adapter: Arc::new(
                crate::channels::slack::delivery::SlackDeliveryAdapter::new(),
            ),
            frontend_url: String::new(),
            api_base_url,
            decisions: Arc::new(everruns_core::DisabledDecisionsService),
            org_decisions: None,
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
            "/v1/channels/{channel_id}/slack/events",
            post(handle_slack_event_channel),
        )
        .route(
            "/v1/e/{channel_id}/slack/events",
            post(handle_slack_event_channel),
        )
        .route(
            "/v1/channels/{channel_id}/slack/manifest",
            get(handle_slack_manifest_channel),
        )
        .route(
            "/v1/e/{channel_id}/slack/manifest",
            get(handle_slack_manifest_channel),
        )
        // EVE-1025. Both spellings, matching the events endpoint: an app
        // created against the legacy URL keeps working without re-saving its
        // manifest.
        .route(
            "/v1/apps/{app_id}/slack/interactivity",
            post(handle_slack_interactivity_legacy),
        )
        .route(
            "/v1/channels/{channel_id}/slack/interactivity",
            post(handle_slack_interactivity_channel),
        )
        .route(
            "/v1/e/{channel_id}/slack/interactivity",
            post(handle_slack_interactivity_channel),
        )
        .with_state(state)
}

pub(crate) enum SlackTarget {
    LegacyApp(String),
    Channel(String),
}

/// Extract text content from an output.message.completed event's data.
/// Delegates to the shared channel runtime.
#[cfg(test)]
fn extract_response_text(data: &serde_json::Value) -> Option<String> {
    everruns_core::channel_runtime::response_text(data)
}

#[cfg(test)]
mod tests_manifest_build;
#[cfg(test)]
mod tests_routing;
#[cfg(test)]
mod tests_signature;
#[cfg(test)]
pub(crate) mod tests_support;
