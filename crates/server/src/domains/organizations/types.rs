// Organizations domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use serde::Serialize;
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
