//! The control plane's `/dp/v1` routes (the data plane calls them, with
//! the link's token), the org token keys, and the org token exchange.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns_contracts::dataplane::{
    AllowanceGrant, AllowanceRequest, FeedAck, FeedBatch, LookupRequest, SealedSecret,
    lookup_secret,
};
use everruns_contracts::host_calls::{self, HostCallRequest, host_call_secret};
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
        .route("/dp/v1/tenants/{org}/host-calls/{name}", post(host_call))
        // The route host calls replaced, kept while cells of the release
        // before roll over. Remove with the release after.
        .route("/dp/v1/tenants/{org}/lookups", post(lookups))
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

/// One host call a tool on the org's data plane makes at call time,
/// answered sealed (`domains/data_plane/host_calls.rs`).
async fn host_call(
    State(state): State<ApiState>,
    Path((org, name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<HostCallRequest>,
) -> Reply<SealedSecret> {
    let call = host_calls::by_name(&name)
        .ok_or((StatusCode::NOT_FOUND, format!("{name} is not a host call")))?;
    answer_call(
        &state,
        &org,
        &headers,
        call,
        request.params,
        &host_call_secret(&request.call),
    )
    .await
}

/// A legacy lookup (`LookupRequest`): the host call its v1 command maps to.
async fn lookups(
    State(state): State<ApiState>,
    Path(org): Path<String>,
    headers: HeaderMap,
    Json(request): Json<LookupRequest>,
) -> Reply<SealedSecret> {
    let call = host_calls::by_command(&request.command).ok_or((
        StatusCode::FORBIDDEN,
        format!("{} is not a data plane lookup", request.command),
    ))?;
    answer_call(
        &state,
        &org,
        &headers,
        call,
        request.params,
        &lookup_secret(&request.request),
    )
    .await
}

/// Run `call` for `org` as the org's internal caller, sealed as `secret_id`.
async fn answer_call(
    state: &ApiState,
    org: &str,
    headers: &HeaderMap,
    call: &host_calls::HostCall,
    params: Value,
    secret_id: &str,
) -> Reply<SealedSecret> {
    let cp = control_plane(state)?;
    require_link(&cp, headers)?;
    let org_id = org_internal(org).ok_or((StatusCode::NOT_FOUND, format!("no org {org}")))?;
    let row = cp.db.dp_org(org_id).await.map_err(internal)?.ok_or((
        StatusCode::NOT_FOUND,
        format!("org {org} is not on a data plane"),
    ))?;
    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org_id,
        &crate::records::FeatureFlagPolicy::current(),
    )
    .await
    .map_err(internal)?;
    // A write runs once per call id (its receipt): a duplicate that still
    // arrives gets the first run's sealed answer, or a refusal while the
    // first has not finished, never a second run.
    let write = call.effect == host_calls::Effect::Write;
    if write {
        use crate::storage::HostCallReceipt;
        match cp
            .db
            .dp_take_host_call_receipt(secret_id, org_id, call.name)
            .await
            .map_err(internal)?
        {
            HostCallReceipt::New => {}
            HostCallReceipt::Answered(sealed) => {
                return serde_json::from_value(sealed).map(Json).map_err(internal);
            }
            HostCallReceipt::Unknown => {
                return Err((
                    StatusCode::CONFLICT,
                    format!(
                        "{} {secret_id} was already received and has not finished; its outcome is unknown",
                        call.name
                    ),
                ));
            }
        }
    }
    let ctx = state.internal_ctx(org_id, flags);
    let answer = dp_domain::host_calls::answer(&cp, &ctx, call, params)
        .await
        .map_err(internal)?;
    let sealed =
        dp_domain::host_calls::seal_answer(&cp, &row, secret_id, &answer).map_err(internal)?;
    if write {
        let stored = serde_json::to_value(&sealed).map_err(internal)?;
        cp.db
            .dp_answer_host_call_receipt(secret_id, &stored)
            .await
            .map_err(internal)?;
    }
    Ok(Json(sealed))
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

/// `POST /v1/auth/token`: exchange the caller's credential for an org token
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
