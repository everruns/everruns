// Machine payment account/policy/attempt HTTP routes.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::payments::types::{
    CreatePaymentAccountRequest, CreatePaymentPolicyRequest, ListPaymentAccountsQuery,
    ListPaymentAttemptsQuery, ListPaymentPoliciesQuery, UpdatePaymentAccountRequest,
    UpdatePaymentPolicyRequest,
};
use crate::domains::payments::{
    CreatePaymentAccount, CreatePaymentPolicy, DisablePaymentAccount, DisablePaymentPolicy,
    GetPaymentAccount, GetPaymentPolicy, ListPaymentAccounts, ListPaymentAttempts,
    ListPaymentPolicies, UpdatePaymentAccountCmd, UpdatePaymentPolicyCmd,
};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::Caller;
use everruns_platform::payment::{PaymentAccount, PaymentAttempt, PaymentPolicy};
use std::sync::Arc;

use super::common::{ErrorResponse, impl_auth_state};

type ApiError = (StatusCode, Json<ErrorResponse>);

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/payments/accounts",
            post(create_payment_account).get(list_payment_accounts),
        )
        .route(
            "/v1/payments/accounts/{payment_account_id}",
            get(get_payment_account)
                .patch(update_payment_account)
                .delete(disable_payment_account),
        )
        .route(
            "/v1/payments/policies",
            post(create_payment_policy).get(list_payment_policies),
        )
        .route(
            "/v1/payments/policies/{payment_policy_id}",
            get(get_payment_policy)
                .patch(update_payment_policy)
                .delete(disable_payment_policy),
        )
        .route("/v1/payments/attempts", get(list_payment_attempts))
        .with_state(state)
}

#[utoipa::path(
    description = "Create a new payment account.",
    post,
    path = "/v1/payments/accounts",
    request_body = CreatePaymentAccountRequest,
    responses(
        (status = 201, description = "Payment account created", body = PaymentAccount),
        (status = 400, description = "Invalid input", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn create_payment_account(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreatePaymentAccountRequest>,
) -> Result<(StatusCode, Json<PaymentAccount>), ApiError> {
    let account = CreatePaymentAccount(req).run(&state.ctx(&org)).await?;
    Ok((StatusCode::CREATED, Json(account)))
}

#[utoipa::path(
    description = "List payment accounts.",
    get,
    path = "/v1/payments/accounts",
    params(ListPaymentAccountsQuery),
    responses((status = 200, description = "List payment accounts", body = Vec<PaymentAccount>)),
    tag = "payments"
)]
pub async fn list_payment_accounts(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListPaymentAccountsQuery>,
) -> Result<Json<Vec<PaymentAccount>>, ApiError> {
    Ok(Json(
        ListPaymentAccounts {
            owner_type: query.owner_type,
            owner_id: query.owner_id,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[utoipa::path(
    description = "Get a payment account by ID.",
    get,
    path = "/v1/payments/accounts/{payment_account_id}",
    params(("payment_account_id" = String, Path, description = "Payment account ID")),
    responses(
        (status = 200, description = "Payment account", body = PaymentAccount),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn get_payment_account(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(payment_account_id): Path<String>,
) -> Result<Json<PaymentAccount>, ApiError> {
    Ok(Json(
        GetPaymentAccount { payment_account_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Update a payment account. Only provided fields are modified.",
    patch,
    path = "/v1/payments/accounts/{payment_account_id}",
    params(("payment_account_id" = String, Path, description = "Payment account ID")),
    request_body = UpdatePaymentAccountRequest,
    responses(
        (status = 200, description = "Payment account updated", body = PaymentAccount),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn update_payment_account(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(payment_account_id): Path<String>,
    Json(req): Json<UpdatePaymentAccountRequest>,
) -> Result<Json<PaymentAccount>, ApiError> {
    Ok(Json(
        UpdatePaymentAccountCmd {
            payment_account_id,
            label: req.label,
            public_address: req.public_address,
            private_key: req.private_key,
            status: req.status,
            metadata: req.metadata,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[utoipa::path(
    description = "Disable a payment account without deleting it.",
    delete,
    path = "/v1/payments/accounts/{payment_account_id}",
    params(("payment_account_id" = String, Path, description = "Payment account ID")),
    responses(
        (status = 204, description = "Payment account disabled"),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn disable_payment_account(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(payment_account_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let result = DisablePaymentAccount { payment_account_id }
        .run(&state.ctx(&org))
        .await?;
    if !result.disabled {
        return Err(ErrorResponse::not_found("Payment account"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    description = "Create a new payment policy.",
    post,
    path = "/v1/payments/policies",
    request_body = CreatePaymentPolicyRequest,
    responses(
        (status = 201, description = "Payment policy created", body = PaymentPolicy),
        (status = 400, description = "Invalid input", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn create_payment_policy(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreatePaymentPolicyRequest>,
) -> Result<(StatusCode, Json<PaymentPolicy>), ApiError> {
    let policy = CreatePaymentPolicy(req).run(&state.ctx(&org)).await?;
    Ok((StatusCode::CREATED, Json(policy)))
}

#[utoipa::path(
    description = "List payment policies.",
    get,
    path = "/v1/payments/policies",
    params(ListPaymentPoliciesQuery),
    responses((status = 200, description = "List payment policies", body = Vec<PaymentPolicy>)),
    tag = "payments"
)]
pub async fn list_payment_policies(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListPaymentPoliciesQuery>,
) -> Result<Json<Vec<PaymentPolicy>>, ApiError> {
    Ok(Json(
        ListPaymentPolicies {
            payment_account_id: query.payment_account_id,
            subject_type: query.subject_type,
            subject_id: query.subject_id,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[utoipa::path(
    description = "Get a payment policy by ID.",
    get,
    path = "/v1/payments/policies/{payment_policy_id}",
    params(("payment_policy_id" = String, Path, description = "Payment policy ID")),
    responses(
        (status = 200, description = "Payment policy", body = PaymentPolicy),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn get_payment_policy(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(payment_policy_id): Path<String>,
) -> Result<Json<PaymentPolicy>, ApiError> {
    Ok(Json(
        GetPaymentPolicy { payment_policy_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Update a payment policy. Only provided fields are modified.",
    patch,
    path = "/v1/payments/policies/{payment_policy_id}",
    params(("payment_policy_id" = String, Path, description = "Payment policy ID")),
    request_body = UpdatePaymentPolicyRequest,
    responses(
        (status = 200, description = "Payment policy updated", body = PaymentPolicy),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn update_payment_policy(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(payment_policy_id): Path<String>,
    Json(req): Json<UpdatePaymentPolicyRequest>,
) -> Result<Json<PaymentPolicy>, ApiError> {
    Ok(Json(
        UpdatePaymentPolicyCmd {
            payment_policy_id,
            allowed_capabilities: req.allowed_capabilities,
            allowed_hosts: req.allowed_hosts,
            rail_preference: req.rail_preference,
            max_amount_usd_per_request: req.max_amount_usd_per_request,
            max_amount_usd_per_turn: req.max_amount_usd_per_turn,
            max_amount_usd_per_day: req.max_amount_usd_per_day,
            require_approval_above_usd: req.require_approval_above_usd,
            status: req.status,
            metadata: req.metadata,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[utoipa::path(
    description = "Disable a payment policy without deleting it.",
    delete,
    path = "/v1/payments/policies/{payment_policy_id}",
    params(("payment_policy_id" = String, Path, description = "Payment policy ID")),
    responses(
        (status = 204, description = "Payment policy disabled"),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "payments"
)]
pub async fn disable_payment_policy(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(payment_policy_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let result = DisablePaymentPolicy { payment_policy_id }
        .run(&state.ctx(&org))
        .await?;
    if !result.disabled {
        return Err(ErrorResponse::not_found("Payment policy"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    description = "List recent payment attempts (operator audit view).",
    get,
    path = "/v1/payments/attempts",
    params(ListPaymentAttemptsQuery),
    responses((status = 200, description = "List payment attempts", body = Vec<PaymentAttempt>)),
    tag = "payments"
)]
pub async fn list_payment_attempts(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListPaymentAttemptsQuery>,
) -> Result<Json<Vec<PaymentAttempt>>, ApiError> {
    Ok(Json(
        ListPaymentAttempts {
            session_id: query.session_id,
            limit: query.limit,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::config::AuthConfig;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use serde::de::DeserializeOwned;
    use serde_json::json;
    use tower::ServiceExt;

    fn test_app() -> Router {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption =
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap();
        routes(AppState::new(
            db.clone(),
            Some(Arc::new(encryption)),
            AuthState::builtin(AuthConfig::default(), db),
        ))
    }

    fn request(method: Method, uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn response_json<T: DeserializeOwned>(response: axum::response::Response) -> T {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn create_test_account(app: Router) -> PaymentAccount {
        let response = app
            .oneshot(request(
                Method::POST,
                "/v1/payments/accounts",
                json!({
                    "owner_type": "organization",
                    "owner_id": "org",
                    "rail": "x402_base",
                    "label": "Base USDC",
                    "public_address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                    "private_key": "0x59c6995e998f97a5a0044966f094538447b6f9b16e5c4f0f4f2b331e1a1a7d3f"
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        response_json(response).await
    }

    #[tokio::test]
    async fn account_routes_support_list_get_and_uuid_paths() {
        let app = test_app();
        let account = create_test_account(app.clone()).await;

        let response = app
            .clone()
            .oneshot(request(
                Method::GET,
                "/v1/payments/accounts",
                serde_json::Value::Null,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let accounts: Vec<PaymentAccount> = response_json(response).await;
        assert_eq!(accounts.len(), 1);

        for id in [account.id.to_string(), account.id.uuid().to_string()] {
            let response = app
                .clone()
                .oneshot(request(
                    Method::GET,
                    &format!("/v1/payments/accounts/{id}"),
                    serde_json::Value::Null,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = app
            .oneshot(request(
                Method::DELETE,
                &format!("/v1/payments/accounts/{}", uuid::Uuid::now_v7()),
                serde_json::Value::Null,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn policy_routes_support_get_uuid_paths_and_missing_disable_404() {
        let app = test_app();
        let account = create_test_account(app.clone()).await;

        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/v1/payments/policies",
                json!({
                    "payment_account_id": account.id.to_string(),
                    "subject_type": "org",
                    "subject_id": "org",
                    "allowed_capabilities": ["parallel"],
                    "allowed_hosts": ["parallelmpp.dev"],
                    "rail_preference": ["x402_base"],
                    "max_amount_usd_per_request": 0.01
                }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let policy: PaymentPolicy = response_json(response).await;

        for id in [policy.id.to_string(), policy.id.uuid().to_string()] {
            let response = app
                .clone()
                .oneshot(request(
                    Method::GET,
                    &format!("/v1/payments/policies/{id}"),
                    serde_json::Value::Null,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = app
            .oneshot(request(
                Method::DELETE,
                &format!("/v1/payments/policies/{}", uuid::Uuid::now_v7()),
                serde_json::Value::Null,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
