// Organizations domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub type ListOrganizationsResponse = crate::common_dto::ListResponse<OrganizationResponse>;

/// Response for organization operations
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganizationResponse {
    /// External identifier (org_<32-hex-chars>)
    pub id: String,
    /// Display name
    pub name: String,
    /// Default LLM model for the organization.
    #[schema(value_type = Option<String>)]
    pub default_model_id: Option<everruns_contracts::typed_id::ModelId>,
    /// Optional small chat model, independent of the main chat and decision defaults.
    #[schema(value_type = Option<String>)]
    pub small_model_id: Option<everruns_contracts::typed_id::ModelId>,
    /// Default harness to preselect in the UI.
    #[schema(value_type = Option<String>)]
    pub default_harness_id: Option<everruns_contracts::typed_id::HarnessId>,
    /// Base harness used when session creation omits harness_id.
    #[schema(value_type = Option<String>)]
    pub base_harness_id: Option<everruns_contracts::typed_id::HarnessId>,
    /// Org-level default provider per service (EVE-569), keyed by service kind.
    /// Empty when no org defaults are configured.
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub default_provider_per_service: std::collections::HashMap<
        everruns_contracts::driver_registry::ServiceKind,
        everruns_contracts::typed_id::ProviderId,
    >,
    /// Who answers deployment-owned decision checks: `deployment` (default)
    /// or `organization` (the org's default decision model).
    pub system_decisions: crate::storage::SystemDecisions,
    /// How many agents one AgentID owner may sign in; null means the platform
    /// default.
    pub agentid_agents_per_owner: Option<i32>,
    /// Web search defaults: the provider agents on "Organization default"
    /// use, the Automatic order, whether model provider search is allowed,
    /// the fallback, results per search and blocked domains. Built-in
    /// defaults (Automatic) when the organization never set any.
    pub web_search_defaults: everruns_contracts::runtime::web_search::WebSearchDefaults,
    /// When the organization was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the organization was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// When the org's creator finished or skipped the setup wizard. `null` means
    /// onboarding is still incomplete, which the UI uses to resume the user at
    /// `/orgs/{id}/setup`. Seeded/default and externally-synced orgs are complete.
    pub onboarding_completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Response body for the `resolve_org` operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResolveOrgResponse {
    /// Public ID of the organization that owns the resource.
    pub org_id: String,
    /// Organization name (for UX messaging).
    pub org_name: String,
}

/// Request to update an organization
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateOrganizationRequest {
    /// The display name of the organization.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Acme Corporation")]
    pub name: Option<String>,
    /// Default LLM model for this organization. Must be enabled; pass null to use the platform
    /// default.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<Option<everruns_contracts::typed_id::ModelId>>,
    /// Shared small chat model for lightweight work. Omit to preserve the selection;
    /// pass null to clear it. Does not change the main chat or decision default.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub small_model_id: Option<Option<everruns_contracts::typed_id::ModelId>>,
    /// Default harness to preselect in the UI for new sessions.
    /// Mutually exclusive with `default_harness_name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    #[serde(default, deserialize_with = "double_option")]
    pub default_harness_id: Option<Option<everruns_contracts::typed_id::HarnessId>>,
    /// Alternative to `default_harness_id` — looked up by stable name within the org.
    /// Mutually exclusive with `default_harness_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "conversation")]
    pub default_harness_name: Option<String>,
    /// Base harness to use when a session is started without an explicit harness_id.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000601")]
    #[serde(default, deserialize_with = "double_option")]
    pub base_harness_id: Option<Option<everruns_contracts::typed_id::HarnessId>>,
    /// Org-level default provider per service (EVE-569). Maps a service kind
    /// (`chat`, `embeddings`, `realtime`, `images`, `rerank`) to the provider id
    /// used as that service's default, consulted after an explicit binding and
    /// before the single-active-provider fallback. When present it **replaces**
    /// the whole map; each referenced provider must exist in the org.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<std::collections::HashMap<String, String>>)]
    pub default_provider_per_service: Option<
        std::collections::HashMap<
            everruns_contracts::driver_registry::ServiceKind,
            everruns_contracts::typed_id::ProviderId,
        >,
    >,
    /// Who answers deployment-owned decision checks (guardrail `jev` checks
    /// and the Slack relevance check). `organization` uses the org's default
    /// decision model (`PUT /v1/models/decision-default`) on its own provider
    /// account; when that model is missing or failing, guardrail checks fail
    /// open and Slack stays silent, never falling back to the deployment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_decisions: Option<crate::storage::SystemDecisions>,
    /// How many agents one AgentID owner may sign in to this organization's
    /// Public Chat channels. Pass null to use the platform default (5).
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<i32>, example = 5, minimum = 0)]
    pub agentid_agents_per_owner: Option<Option<i32>>,
    /// Web search defaults; replaces the stored defaults as a whole. Omitted
    /// fields take their built-in default. `provider` cannot be
    /// `org_default`, `account_id` and `automatic_order` entries must name
    /// this organization's search accounts, free search is always last in
    /// the Automatic order, and `results_per_search` is 1 to 20.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search_defaults: Option<everruns_contracts::runtime::web_search::WebSearchDefaults>,
}

fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}
