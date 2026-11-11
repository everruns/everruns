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
use crate::auth::middleware::AuthUser;
use crate::domains::data_plane::{self as dp_domain, ControlPlane, Link, org_internal};

type Reply<T> = Result<Json<T>, (StatusCode, String)>;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/dp/v1/feed", post(feed))
        .route("/dp/v1/tenants/{org}/allowances", post(allowances))
        .route("/.well-known/jwks.json", get(jwks))
        .route("/v1/auth/token", post(token))
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

/// The link's token, when one is configured: a data plane must send it.
fn require_link(cp: &ControlPlane, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    let Some(token) = &cp.link.token else {
        return Ok(());
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
    let cp = control_plane(&state)?;
    require_link(&cp, &headers)?;
    let budgets = crate::domains::budgets::BudgetService::new(state.db.clone());
    dp_domain::feed::ingest(&cp, &budgets, batch)
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

#[derive(Deserialize)]
struct TokenRequest {
    /// The org's public id.
    org: String,
}

/// `POST /v1/auth/token`: exchange the caller's credential for an org token
/// of an org they belong to that runs on a data plane.
async fn token(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(body): Json<TokenRequest>,
) -> Reply<Value> {
    let membership = user
        .get_org(&body.org)
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
        "org": body.org,
    })))
}
