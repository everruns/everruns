// Inbound channel surfaces (webhooks, A2A, API endpoint keys, AG-UI, FCP,
// Public Chat, Poppy): their states, per-channel rate limiters and routes.
//
// Decision: each channel gets its own rate-limiter namespace so one channel's
//   public traffic can never share or exhaust another channel's per-channel cap.
//   With Valkey configured, counters are shared across instances.

use crate::api;
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::api::sse::SseConnectionTracker;
use crate::auth;
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{EncryptionService, StorageBackend};
use crate::valkey::ValkeyClient;
use std::sync::Arc;

pub(super) struct ChannelDeps {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub runner: Arc<dyn everruns_core::host::TurnBackend>,
    pub event_delivery: EventDelivery,
    pub sse_tracker: Arc<SseConnectionTracker>,
    pub valkey: Option<ValkeyClient>,
    pub auth: auth::AuthState,
    pub frontend_url: String,
    pub public_chat_enabled: bool,
    pub mcp_event_triggers: Arc<crate::domains::agent_triggers::McpEventTriggers>,
}

pub(super) struct ChannelStates {
    pub webhooks: api::channel_webhooks::ChannelWebhookState,
    pub a2a: crate::channels::a2a::ChannelA2aState,
    pub api: api::channel_api::ChannelApiState,
    pub ag_ui: crate::channels::ag_ui::AgUiState,
    pub fcp: crate::channels::fcp::FcpState,
    pub public_chat: crate::channels::ag_ui::AgUiState,
    pub poppy: crate::channels::poppy::PoppyState,
}

impl ChannelDeps {
    fn rate_limiter(&self, namespace: &'static str) -> ChannelRateLimiter {
        match self.valkey.clone() {
            Some(client) => ChannelRateLimiter::with_valkey(namespace, client),
            None => ChannelRateLimiter::in_memory(namespace),
        }
    }
}

impl ChannelStates {
    pub(super) fn build(deps: ChannelDeps) -> Self {
        let webhooks = api::channel_webhooks::ChannelWebhookState::new(
            deps.db.clone(),
            deps.encryption.clone(),
            deps.runner.clone(),
            deps.event_delivery.clone(),
            deps.rate_limiter("webhook"),
        )
        .with_mcp_event_triggers(deps.mcp_event_triggers.clone());
        let a2a_replay_store = match deps.valkey.clone() {
            Some(client) => crate::channels::a2a::signing::A2aReplayStore::with_valkey(client),
            None => crate::channels::a2a::signing::A2aReplayStore::in_memory(),
        };
        let a2a = crate::channels::a2a::ChannelA2aState::new(
            deps.db.clone(),
            deps.encryption.clone(),
            deps.runner.clone(),
            deps.event_delivery.clone(),
            deps.sse_tracker.clone(),
            deps.rate_limiter("a2a"),
            a2a_replay_store.clone(),
            deps.frontend_url.clone(),
        );
        let poppy = crate::channels::poppy::PoppyState::new(
            deps.db.clone(),
            deps.encryption.clone(),
            deps.runner.clone(),
            deps.event_delivery.clone(),
            deps.rate_limiter("poppy"),
            a2a_replay_store,
        );
        // api_endpoint execution keys.
        let api = api::channel_api::ChannelApiState::new(
            deps.db.clone(),
            deps.encryption.clone(),
            deps.runner.clone(),
            deps.event_delivery.clone(),
            deps.sse_tracker.clone(),
            deps.rate_limiter("apikey"),
        )
        .with_runtime_auth(deps.auth.clone());
        let ag_ui = crate::channels::ag_ui::AgUiState::new(
            deps.db.clone(),
            deps.encryption.clone(),
            deps.runner.clone(),
            deps.event_delivery.clone(),
            deps.sse_tracker.clone(),
            deps.rate_limiter("agui"),
        )
        .with_runtime_auth(deps.auth.clone());
        let fcp = crate::channels::fcp::FcpState::new(
            deps.db.clone(),
            deps.encryption.clone(),
            deps.runner.clone(),
            deps.event_delivery.clone(),
            deps.rate_limiter("fcp"),
        );
        // Public Chat's traffic is anonymous, which is why it gets a namespace
        // separate from AG-UI even though it shares the AG-UI state.
        let public_chat = crate::channels::ag_ui::AgUiState::new(
            deps.db.clone(),
            deps.encryption.clone(),
            deps.runner.clone(),
            deps.event_delivery.clone(),
            deps.sse_tracker.clone(),
            deps.rate_limiter("public_chat"),
        )
        .with_public_chat_enabled(deps.public_chat_enabled)
        .with_runtime_auth(deps.auth.clone());
        Self {
            webhooks,
            a2a,
            api,
            ag_ui,
            fcp,
            public_chat,
            poppy,
        }
    }

    /// The channels' routes: those under the API prefix, and those at the
    /// server root (Poppy's RFC 8414 metadata).
    pub(super) fn into_routes(self) -> (axum::Router, axum::Router) {
        let api = axum::Router::new()
            .merge(api::channel_webhooks::routes(self.webhooks))
            .merge(crate::channels::a2a::routes(self.a2a))
            .merge(api::channel_api::routes(self.api))
            .merge(crate::channels::ag_ui::routes(self.ag_ui))
            .merge(crate::channels::public_chat::routes(self.public_chat))
            .merge(crate::channels::fcp::routes(self.fcp))
            .merge(crate::channels::poppy::routes(self.poppy.clone()));
        let root = crate::channels::poppy::well_known_routes(self.poppy);
        (api, root)
    }
}
