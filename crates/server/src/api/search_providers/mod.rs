//! Web search providers for the active organization: the catalog the Web
//! search page lists, and a plan preview for the agent editor's "How this
//! agent will search" box (with an optional test query for a card's test
//! button).
//!
//! Decision: the preview resolves with the same `resolve_plan` the running
//! tool uses, so the box cannot drift from what a search does. It cannot see
//! session or user Brave keys, so a `brave` step reports which organization
//! account it would fall back to. A test query runs through the runtime egress
//! policy, like an agent's search, and needs an organization admin because it
//! spends the account's quota. See knowledge/integrations/web-search.md.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::web_search::{
    NATIVE_WEB_SEARCH_PRICE_NOTE, SearchContext, SearchDriver, SearchFallbackMode, SearchHit,
    SearchProviderSetting, SearchRequest, WEB_SEARCH_ACCOUNT_PROVIDERS,
    WEB_SEARCH_CONNECTOR_CAPABILITY, WebSearchDefaults, WebSearchOrgSettings,
    native_web_search_supported,
};
use everruns_core::OrgRole;
use everruns_integrations::web_search::{
    BraveSearchDriver, FreeSearchDriver, PlanEntry, PlanInputs, PlanStep, WebSearchConfig,
    resolve_plan, run_plan,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::ErrorResponse;
use crate::auth::ResolvedOrg;

pub type AppState = super::virtual_user_connections::AppState;

type ApiError = (StatusCode, Json<ErrorResponse>);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/search-providers", get(list_search_providers))
        .route("/v1/search-providers/preview", post(preview_search_plan))
        .with_state(state)
}

/// A provider that needs no setup.
#[derive(Debug, Serialize, ToSchema)]
pub struct BuiltInSearchProvider {
    /// `free` or `native`.
    #[schema(example = "free")]
    pub id: String,
    pub name: String,
    pub description: String,
    /// Plain-words cost, when the provider bills per search.
    pub price_note: Option<String>,
}

/// A connected model provider whose models search natively.
#[derive(Debug, Serialize, ToSchema)]
pub struct NativeSearchProvider {
    #[schema(value_type = String)]
    pub provider_id: everruns_contracts::typed_id::ProviderId,
    pub name: String,
    #[schema(example = "anthropic")]
    pub provider_type: String,
    pub price_note: String,
}

/// A connector whose organization accounts answer web search.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchConnector {
    /// Connector id, used to connect an account at
    /// `/v1/orgs/{org}/organization-connections/providers/{provider}`.
    #[schema(example = "brave_search")]
    pub provider: String,
    pub name: String,
    pub description: String,
    pub icon: String,
}

/// A connected organization search account (never its key).
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchAccountSummary {
    pub id: Uuid,
    #[schema(example = "brave_search")]
    pub provider: String,
    /// Account name the admin gave it.
    pub name: String,
    /// `ready` when a key is stored, `missing_key` otherwise.
    #[schema(example = "ready")]
    pub status: String,
    /// Place in the Automatic order (1-based), counting native and accounts.
    pub automatic_position: Option<usize>,
    pub connected_at: DateTime<Utc>,
}

/// Everything the organization can search with.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchProviderCatalog {
    pub built_in: Vec<BuiltInSearchProvider>,
    pub native_providers: Vec<NativeSearchProvider>,
    pub connectors: Vec<SearchConnector>,
    pub accounts: Vec<SearchAccountSummary>,
    pub defaults: WebSearchDefaults,
}

/// Run one query against one provider.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchTestQuery {
    #[schema(example = "tokio release notes")]
    pub query: String,
    /// `free` or `account:<id>`.
    #[schema(example = "free")]
    pub provider: String,
}

/// Resolve an agent's web search setting and model to a plan.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchPlanPreviewRequest {
    /// The agent's `web_search` capability config; omitted means
    /// `{"provider": "org_default"}`.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
    /// The model the agent runs; omitted uses the organization's default model.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub model_id: Option<everruns_contracts::typed_id::ModelId>,
    /// Optional test query against one provider (organization admins).
    #[serde(default)]
    pub test: Option<SearchTestQuery>,
}

/// One provider in a previewed plan.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchPlanStep {
    /// `native`, `brave`, `account:<id>` or `free`.
    pub id: String,
    pub label: String,
    /// Whether this step can answer for this model now.
    pub available: bool,
    /// Why it cannot, or what it relies on.
    pub note: Option<String>,
    pub price_note: Option<String>,
}

/// One result of a test query.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchTestResult {
    pub title: String,
    pub url: String,
    pub site: String,
    pub snippet: Option<String>,
}

/// What a test query returned.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchTestOutcome {
    pub provider: String,
    pub ok: bool,
    pub duration_ms: u64,
    pub message: Option<String>,
    pub results: Vec<SearchTestResult>,
}

/// The plan an agent's search would follow.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchPlanPreview {
    /// The setting in effect after following the organization default.
    pub provider: SearchProviderSetting,
    pub account_id: Option<Uuid>,
    pub fallback: SearchFallbackMode,
    /// Results per search when the model does not ask.
    pub limit: u32,
    /// Whether the model searches natively (and the organization allows it).
    pub model_has_native: bool,
    pub org_blocked_domains: Vec<String>,
    pub steps: Vec<SearchPlanStep>,
    pub test: Option<SearchTestOutcome>,
}

fn internal(what: &'static str) -> impl FnOnce(anyhow::Error) -> ApiError {
    move |error| {
        tracing::error!(%error, "{what}");
        ErrorResponse::new("Internal error").into_response(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

fn bad_request(message: impl Into<String>) -> ApiError {
    ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST)
}

/// The org's search account rows (with key presence) and its settings.
async fn org_settings(
    state: &AppState,
    org_id: i64,
) -> Result<
    (
        Vec<crate::storage::VirtualUserConnectionRow>,
        WebSearchOrgSettings,
    ),
    ApiError,
> {
    let rows: Vec<_> = state
        .db
        .list_organization_connections(org_id)
        .await
        .map_err(internal("Failed to list search accounts"))?
        .into_iter()
        .filter(|row| WEB_SEARCH_ACCOUNT_PROVIDERS.contains(&row.provider.as_str()))
        .collect();
    let settings =
        crate::domains::organizations::web_search::web_search_settings(&state.db, org_id)
            .await
            .map_err(internal("Failed to load web search settings"))?;
    Ok((rows, settings))
}

#[utoipa::path(
    get,
    path = "/v1/search-providers",
    tag = "web-search",
    description = "List the web search providers of the active organization: built-in free and model provider search, connected model providers that search natively, connectors that hold search accounts, connected search accounts with status, and the organization's web search defaults.",
    responses((status = 200, body = SearchProviderCatalog))
)]
pub async fn list_search_providers(
    State(state): State<AppState>,
    org: ResolvedOrg,
) -> Result<Json<SearchProviderCatalog>, ApiError> {
    let (rows, settings) = org_settings(&state, org.org_id).await?;
    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org.org_id,
        &state.auth.feature_flag_policy,
    )
    .await
    .unwrap_or_default();
    let native_providers = state
        .db
        .list_providers(org.org_id)
        .await
        .map_err(internal("Failed to list model providers"))?
        .into_iter()
        .filter(|provider| native_web_search_supported(&provider.provider_type))
        .map(|provider| NativeSearchProvider {
            provider_id: provider.id,
            name: provider.name,
            provider_type: provider.provider_type,
            price_note: NATIVE_WEB_SEARCH_PRICE_NOTE.to_string(),
        })
        .collect();
    let connectors = state
        .connectors
        .list()
        .into_iter()
        .filter(|connector| {
            connector
                .capabilities()
                .contains(&WEB_SEARCH_CONNECTOR_CAPABILITY)
                && flags.is_connector_enabled(connector.provider_id())
        })
        .map(|connector| SearchConnector {
            provider: connector.provider_id().to_string(),
            name: connector.display_name().to_string(),
            description: connector.description().to_string(),
            icon: connector.icon().to_string(),
        })
        .collect();
    let automatic = automatic_ids(&settings);
    let accounts = rows
        .into_iter()
        .map(|row| SearchAccountSummary {
            automatic_position: automatic
                .iter()
                .position(|id| *id == format!("account:{}", row.id))
                .map(|index| index + 1),
            status: if row.access_token_encrypted.is_some() {
                "ready"
            } else {
                "missing_key"
            }
            .to_string(),
            name: row.name.clone().unwrap_or_else(|| row.provider.clone()),
            id: row.id,
            provider: row.provider,
            connected_at: row.created_at,
        })
        .collect();
    Ok(Json(SearchProviderCatalog {
        built_in: vec![
            BuiltInSearchProvider {
                id: "free".into(),
                name: "Free search".into(),
                description: "Keyless best-effort web search (DuckDuckGo, then Mojeek). The last \
                              fallback of every plan."
                    .into(),
                price_note: None,
            },
            BuiltInSearchProvider {
                id: "native".into(),
                name: "Model provider search".into(),
                description: "The search of the model running the turn, when it has one.".into(),
                price_note: Some(NATIVE_WEB_SEARCH_PRICE_NOTE.into()),
            },
        ],
        native_providers,
        connectors,
        accounts,
        defaults: settings.defaults,
    }))
}

/// Plan ids of the Automatic order for a model with native search.
fn automatic_ids(settings: &WebSearchOrgSettings) -> Vec<String> {
    let config = WebSearchConfig {
        provider: SearchProviderSetting::Auto,
        fallback: Some(SearchFallbackMode::Next),
        ..WebSearchConfig::default()
    };
    resolve_plan(PlanInputs {
        config: &config,
        org: Some(settings),
        model_has_native: true,
        brave_key: false,
    })
    .ids()
}

/// Whether the model (or the org's default model) searches natively.
async fn model_has_native(
    state: &AppState,
    org_id: i64,
    model_id: Option<everruns_contracts::typed_id::ModelId>,
) -> Result<bool, ApiError> {
    let model_id = match model_id {
        Some(id) => Some(id),
        None => state
            .db
            .get_organization_settings(org_id)
            .await
            .map_err(internal("Failed to load organization settings"))?
            .and_then(|settings| settings.default_model_id),
    };
    let Some(model_id) = model_id else {
        return Ok(false);
    };
    let model = state
        .db
        .get_model_with_provider(org_id, model_id.uuid())
        .await
        .map_err(internal("Failed to load model"))?
        .ok_or_else(|| bad_request("Model not found"))?;
    Ok(native_web_search_supported(&model.provider_type))
}

#[utoipa::path(
    post,
    path = "/v1/search-providers/preview",
    tag = "web-search",
    description = "Resolve an agent's web search setting and model to the ordered plan its searches follow, with the organization's current defaults. Optionally run a test query against one provider (`free` or `account:<id>`; organization admins only).",
    request_body = SearchPlanPreviewRequest,
    responses(
        (status = 200, body = SearchPlanPreview),
        (status = 400, description = "Invalid config, unknown model, or unknown test provider", body = ErrorResponse),
        (status = 403, description = "Test queries need an organization admin", body = ErrorResponse)
    )
)]
pub async fn preview_search_plan(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Json(body): Json<SearchPlanPreviewRequest>,
) -> Result<Json<SearchPlanPreview>, ApiError> {
    let config = WebSearchConfig::from_value(&body.config).map_err(bad_request)?;
    if body
        .test
        .as_ref()
        .is_some_and(|_| !org.role.has_permission(OrgRole::Admin))
    {
        return Err(
            ErrorResponse::new("Test queries need an organization admin")
                .into_response(StatusCode::FORBIDDEN),
        );
    }
    let (rows, settings) = org_settings(&state, org.org_id).await?;
    let model_native = model_has_native(&state, org.org_id, body.model_id).await?;
    let plan = resolve_plan(PlanInputs {
        config: &config,
        org: Some(&settings),
        model_has_native: model_native,
        brave_key: false,
    });
    let ready = |id: Uuid| {
        rows.iter()
            .any(|row| row.id == id && row.access_token_encrypted.is_some())
    };
    let account_label = |id: Uuid| {
        settings
            .accounts
            .iter()
            .find(|account| account.id == id)
            .map(|account| format!("Brave Search · {}", account.name))
    };
    let steps = plan
        .entries
        .iter()
        .map(|entry| match entry {
            PlanEntry::Native => SearchPlanStep {
                id: entry.id(),
                label: "Model provider search".into(),
                available: model_native && plan.allow_native,
                note: if !plan.allow_native {
                    Some("Model provider search is turned off for this organization".into())
                } else if !model_native {
                    Some("This model has no search of its own".into())
                } else {
                    None
                },
                price_note: Some(NATIVE_WEB_SEARCH_PRICE_NOTE.into()),
            },
            PlanEntry::Brave => {
                let fallback = settings
                    .accounts
                    .iter()
                    .find(|account| account.provider == "brave_search" && ready(account.id));
                SearchPlanStep {
                    id: entry.id(),
                    label: "Brave Search".into(),
                    available: fallback.is_some(),
                    note: Some(match fallback {
                        Some(account) => format!(
                            "Uses a session or user Brave key, else the organization account {}",
                            account.name
                        ),
                        None => "Needs a session or user Brave key; the organization has no \
                                 Brave account"
                            .into(),
                    }),
                    price_note: None,
                }
            }
            PlanEntry::Account { id } => SearchPlanStep {
                id: entry.id(),
                label: account_label(*id).unwrap_or_else(|| "Search account".into()),
                available: ready(*id),
                note: (!ready(*id))
                    .then(|| "The search account is not connected in this organization".into()),
                price_note: None,
            },
            PlanEntry::Free => SearchPlanStep {
                id: entry.id(),
                label: "Free search".into(),
                available: true,
                note: None,
                price_note: None,
            },
        })
        .collect();
    let test = match body.test {
        Some(test) => Some(run_test(&state, &rows, test).await?),
        None => None,
    };
    Ok(Json(SearchPlanPreview {
        provider: plan.provider,
        account_id: plan.account_id,
        fallback: plan.fallback,
        limit: plan.limit,
        model_has_native: model_native && plan.allow_native,
        org_blocked_domains: plan.org_blocked_domains,
        steps,
        test,
    }))
}

async fn run_test(
    state: &AppState,
    rows: &[crate::storage::VirtualUserConnectionRow],
    test: SearchTestQuery,
) -> Result<SearchTestOutcome, ApiError> {
    let query = test.query.trim();
    if query.is_empty() {
        return Err(bad_request("Test query must not be empty"));
    }
    let driver: Arc<dyn SearchDriver> = if test.provider == "free" {
        Arc::new(FreeSearchDriver::default())
    } else {
        let id = test
            .provider
            .strip_prefix("account:")
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| bad_request("Test provider must be free or account:<id>"))?;
        let row = rows
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| bad_request("Search account not found"))?;
        let encrypted = row
            .access_token_encrypted
            .as_deref()
            .ok_or_else(|| bad_request("The search account has no key"))?;
        let key = state
            .encryption
            .as_ref()
            .ok_or_else(|| {
                ErrorResponse::new("Encryption not configured")
                    .into_response(StatusCode::INTERNAL_SERVER_ERROR)
            })?
            .decrypt_to_string(encrypted)
            .map_err(internal("Failed to decrypt search account key"))?;
        Arc::new(BraveSearchDriver::new(key))
    };
    let context = SearchContext {
        egress: Arc::new(everruns_core::host::DirectEgressService::for_runtime_traffic_from_env()),
        network_access: None,
    };
    let outcome = run_plan(
        &[PlanStep::Run(driver)],
        &SearchRequest::new(query),
        &context,
    )
    .await;
    let attempt = outcome.attempts.into_iter().next();
    Ok(SearchTestOutcome {
        provider: test.provider,
        ok: outcome.response.is_some(),
        duration_ms: attempt.as_ref().map_or(0, |attempt| attempt.duration_ms),
        message: attempt.and_then(|attempt| attempt.message),
        results: outcome
            .response
            .map(|response| response.hits.iter().map(test_result).collect())
            .unwrap_or_default(),
    })
}

fn test_result(hit: &SearchHit) -> SearchTestResult {
    SearchTestResult {
        title: hit.title.clone(),
        url: hit.url.clone(),
        site: hit.site.clone(),
        snippet: hit.snippet.clone(),
    }
}

#[cfg(test)]
mod tests;
