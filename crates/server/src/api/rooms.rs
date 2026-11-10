// Rooms HTTP routes: `/v1/rooms` (knowledge/runtime-resources/rooms.md).
//
// Every route is a domain command. The router carries the session and message
// services because posting a message wakes the agent members it addresses.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::messages::MessageService;
use crate::domains::rooms::*;
use crate::domains::sessions::SessionService;
use crate::services::CapabilityService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::Router;
use everruns_core::Caller;
use std::sync::Arc;

use super::command_http::CommandRouterExt;
use super::common::impl_auth_state;
use super::dispatch::impl_dispatchable;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    /// Checks agents set to wake on relevant messages.
    pub relevance: Option<Arc<crate::services::relevance::RelevanceChecker>>,
}

impl AppState {
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_session_service(self.session_service.clone())
        .with_message_service(self.message_service.clone())
        .with_relevance(self.relevance.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// The rooms routes on the shared API services, waking agents through the
/// message routes' session and message services.
pub fn routes_from(api: &super::state::ApiState, messages: &super::messages::AppState) -> Router {
    routes(AppState {
        db: api.db.clone(),
        encryption: api.encryption.clone(),
        capability_service: api.capability_service.clone(),
        auth: api.auth.clone(),
        session_service: messages.session_service.clone(),
        message_service: messages.message_service.clone(),
        relevance: None,
    })
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        // The org-wide feed is registered before `/v1/rooms/{room_id}` reads.
        .command::<ListOrgRoomEvents>()
        .command::<CreateRoom>()
        .command::<ListRooms>()
        .command::<GetRoom>()
        .command::<UpdateRoom>()
        .command::<ArchiveRoom>()
        .command::<AddRoomMember>()
        .command::<ListRoomMembers>()
        .command::<UpdateRoomMember>()
        .command::<RemoveRoomMember>()
        .command::<PostRoomMessage>()
        .command::<ListRoomMessages>()
        .command::<GetRoomMessage>()
        .command::<EditRoomMessage>()
        .command::<DeleteRoomMessage>()
        .command::<ListRoomReplies>()
        .command::<ReactToRoomMessage>()
        .command::<PinRoomMessage>()
        .command::<ListRoomPins>()
        .command::<SearchRoomMessages>()
        .command::<MarkRoomRead>()
        .command::<ReadRoomUnread>()
        .command::<ListRoomMessageReaders>()
        .command::<ListRoomEvents>()
        .command::<ListRoomFiles>()
        .command::<ReadRoomFile>()
        .command::<WriteRoomFile>()
        .command::<EditRoomFile>()
        .command::<MoveRoomFile>()
        .command::<DeleteRoomFile>()
        .command::<ListRoomFileHistory>()
        .command::<RestoreRoomFile>()
        .with_state(state)
}
