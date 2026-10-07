//! The org setting naming who answers deployment-owned decision checks.

/// Who answers an org's deployment-owned decision checks (guardrail `jev`
/// checks and the Slack relevance check).
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SystemDecisions {
    /// The deployment's decisions service.
    #[default]
    Deployment,
    /// The org's decision default model, on the org's own provider account.
    Organization,
}

impl SystemDecisions {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deployment => "deployment",
            Self::Organization => "organization",
        }
    }

    /// Read the stored value. The column's CHECK constraint admits only the
    /// two names, so anything else is treated as the safe default.
    pub fn from_db(value: &str) -> Self {
        if value == "organization" {
            Self::Organization
        } else {
            Self::Deployment
        }
    }
}
