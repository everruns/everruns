use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use everruns_core::Caller;
use everruns_durable::WorkflowEventStore;
use everruns_platform::App;

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::apps::types::ListAppsQuery;
use crate::domains::common::Command;
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::services::CapabilityService;
use crate::storage::{EncryptionService, StorageBackend};

use super::common::{ApiResult, ListResponse, UrlBuilder, WithUrls, impl_auth_state};
use super::dispatch::{Dispatchable, impl_dispatchable};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub session_service: Option<Arc<SessionService>>,
    pub message_service: Option<Arc<MessageService>>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        workflow_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            encryption,
            workflow_store,
            capability_service,
            auth,
            session_service: None,
            message_service: None,
        }
    }

    pub fn with_invocation_services(
        mut self,
        session_service: Arc<SessionService>,
        message_service: Arc<MessageService>,
    ) -> Self {
        self.session_service = Some(session_service);
        self.message_service = Some(message_service);
        self
    }

    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        let mut ctx = crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_workflow_store(self.workflow_store.clone());
        if let Some(session_service) = &self.session_service {
            ctx = ctx.with_session_service(session_service.clone());
        }
        if let Some(message_service) = &self.message_service {
            ctx = ctx.with_message_service(message_service.clone());
        }
        ctx
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/apps", get(list_apps))
        .route("/v1/apps/{app_id}", get(get_app))
        .with_state(state)
}

#[utoipa::path(
    description = "List archival App records. This endpoint is read-only and deprecated.",
    get,
    path = "/v1/apps",
    responses(
        (status = 200, description = "Archival list of Apps", body = ListResponse<WithUrls<App>>),
        (status = 500, description = "Internal server error")
    ),
    params(ListAppsQuery),
    tag = "apps"
)]
pub async fn list_apps(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAppsQuery>,
) -> ApiResult<ListResponse<WithUrls<App>>> {
    let apps = crate::domains::apps::ListApps {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
    }
    .run(&state.ctx(&org))
    .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(apps).with_urls(&builder)))
}

#[utoipa::path(
    description = "Get an archival App record. This endpoint is read-only and deprecated.",
    get,
    path = "/v1/apps/{app_id}",
    params(("app_id" = String, Path, description = "App ID")),
    responses(
        (status = 200, description = "Archival App record", body = WithUrls<App>),
        (status = 400, description = "Invalid App ID"),
        (status = 404, description = "App not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "apps"
)]
pub async fn get_app(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
) -> ApiResult<WithUrls<App>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::apps::GetApp { id: app_id })
        .await
}
