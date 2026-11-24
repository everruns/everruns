//! The control plane's `/dp/v1` routes (the data plane calls them, with
//! the link's token), the org token keys, and the org token exchange.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns_contracts::dataplane::{AllowanceGrant, AllowanceRequest, FeedAck, FeedBatch};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::state::ApiState;
use crate::auth::middleware::{AuthUser, ResolvedOrg};
use crate::domains::data_plane::{self as dp_domain, ControlPlane, Link, org_internal};

type Reply<T> = Result<Json<T>, (StatusCode, String)>;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/dp/v1/feed", post(feed))
        .route("/dp/v1/tenants/{org}/allowances", post(allowances))
        .route("/.well-known/jwks.json", get(jwks))
        .route("/v1/platform/auth/token", post(token))
        .with_state(state)
}

fn control_plane(state: &ApiState) -> Result<ControlPlane, (StatusCode, String)> {
    let link = Link::from_env().map_err(internal)?;
    Ok(ControlPlane::new(
        state.db.clone(),
        state.encryption.clone(),
        link,
    ))
}

fn internal(error: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::warn!(%error, "data plane call failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "the control plane could not answer".into(),
    )
}

/// The link's token: a data plane must send it.
///
/// Decision: a control plane with no link token configured refuses every
/// `/dp/v1` call instead of serving it to anyone. These routes sit under the
/// public API, so an unconfigured server would otherwise take feed batches,
/// allowance requests and lookups from the internet.
fn require_link(cp: &ControlPlane, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    check_link(cp.link.token.as_deref(), headers)
}

fn check_link(token: Option<&str>, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let Some(token) = token else {
        return Err((
            StatusCode::NOT_FOUND,
            "this control plane has no data plane link".into(),
        ));
    };
    let sent = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    let same = sent.len() == token.len()
        && sent
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if same {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            "the data plane token is missing or wrong".into(),
        ))
    }
}

async fn feed(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(batch): Json<FeedBatch>,
) -> Reply<FeedAck> {
    let mut cp = control_plane(&state)?;
    if let Some(egress) = state.egress_service() {
        cp = cp.with_egress(egress);
    }
    require_link(&cp, &headers)?;
    let budgets = crate::domains::budgets::BudgetService::new(state.db.clone());
    dp_domain::feed::ingest(&cp, &budgets, state.event_listeners(), batch)
        .await
        .map(Json)
        .map_err(internal)
}

async fn allowances(
    State(state): State<ApiState>,
    Path(org): Path<String>,
    headers: HeaderMap,
    Json(request): Json<AllowanceRequest>,
) -> Reply<AllowanceGrant> {
    let cp = control_plane(&state)?;
    require_link(&cp, &headers)?;
    let org_id = org_internal(&org).ok_or((StatusCode::NOT_FOUND, format!("no org {org}")))?;
    if cp.db.dp_org(org_id).await.map_err(internal)?.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("org {org} is not on a data plane"),
        ));
    }
    dp_domain::allowances::answer(&cp, org_id, &request)
        .await
        .map(Json)
        .map_err(internal)
}

async fn jwks(State(state): State<ApiState>) -> Reply<Value> {
    let cp = control_plane(&state)?;
    dp_domain::tokens::jwks(&cp)
        .await
        .map(Json)
        .map_err(internal)
}

#[derive(Deserialize, Default)]
struct TokenRequest {
    /// The org's public id; the request's org (`X-Org-Id` or the org
    /// cookie) when absent, which is how the edge exchanges a console
    /// session or an API key without knowing the org.
    #[serde(default)]
    org: Option<String>,
}

/// `POST /v1/platform/auth/token`: exchange the caller's credential for an org token
/// of an org they belong to that runs on a data plane.
async fn token(
    State(state): State<ApiState>,
    user: AuthUser,
    resolved: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
    Json(body): Json<TokenRequest>,
) -> Reply<Value> {
    let org = match body.org {
        Some(org) => org,
        None => {
            resolved
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "name the organization (org, or the X-Org-Id header)".to_string(),
                    )
                })?
                .public_id
        }
    };
    let membership = user
        .get_org(&org)
        .ok_or((StatusCode::NOT_FOUND, "Organization not found".to_string()))?;
    let cp = control_plane(&state)?;
    if cp
        .db
        .dp_org(membership.org_id)
        .await
        .map_err(internal)?
        .is_none()
    {
        return Err((
            StatusCode::CONFLICT,
            "the organization does not run on a data plane".into(),
        ));
    }
    let (token, exp) = dp_domain::tokens::issue(&cp, membership.org_id, &user.id.to_string())
        .await
        .map_err(internal)?;
    Ok(Json(json!({
        "token": token,
        "token_type": "Bearer",
        "expires_at_unix": exp,
        "org": org,
    })))
}

/// The calls of the injected servers this control plane serves
/// (`injected/hosted.rs`, `domains/data_plane/calls.rs`), as the org's
/// internal caller with the org's feature flags, for an org on a data
/// plane only.
pub struct InjectedCalls(pub ApiState);

#[async_trait::async_trait]
impl crate::injected::hosted::CallRunner for InjectedCalls {
    async fn answer(
        &self,
        org: everruns_contracts::typed_id::OrgId,
        call: &'static dp_domain::calls::Call,
        params: Value,
    ) -> anyhow::Result<dp_domain::calls::Answer> {
        let state = &self.0;
        let cp = control_plane(state).map_err(|(_, message)| anyhow::anyhow!(message))?;
        let org_id = everruns_core::organization::org_internal_id_from_public(org);
        anyhow::ensure!(
            cp.db.dp_org(org_id).await?.is_some(),
            "org {org} is not on a data plane"
        );
        let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
            &state.db,
            org_id,
            &crate::records::FeatureFlagPolicy::current(),
        )
        .await?;
        let ctx = state.internal_ctx(org_id, flags);
        dp_domain::calls::answer(&cp, &ctx, call, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    #[test]
    fn a_control_plane_without_a_link_token_serves_no_data_plane_call() {
        let refused = check_link(None, &HeaderMap::new()).unwrap_err();
        assert_eq!(refused.0, StatusCode::NOT_FOUND);
        assert!(check_link(None, &bearer("anything")).is_err());
    }

    #[test]
    fn a_data_plane_call_needs_the_links_own_token() {
        assert!(check_link(Some("s3cret"), &bearer("s3cret")).is_ok());
        for headers in [HeaderMap::new(), bearer("s3cre"), bearer("s3cret!")] {
            let refused = check_link(Some("s3cret"), &headers).unwrap_err();
            assert_eq!(refused.0, StatusCode::UNAUTHORIZED);
        }
    }
}
