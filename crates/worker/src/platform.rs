//! Worker runtime platform helpers.
//!
//! Workers honor the same `HostComposition` shape as the server so
//! embedders can keep execution and control-plane runtime surfaces aligned.
//! The worker default only includes capabilities and LLM drivers because
//! connection providers and harness templates are server-owned by default.

use everruns_core::DeploymentGrade;
use everruns_host::DirectEgressService;
use everruns_host::{HostComposition, SystemUtilityLlmConfig};
use everruns_integrations_typesafe::SystemDecisionsConfig;
use std::sync::Arc;

/// Build the default worker-side platform definition for the current deployment grade.
pub fn default_host_composition() -> HostComposition {
    default_host_composition_for_grade(DeploymentGrade::from_env())
}

/// Build the default worker-side platform definition for an explicit grade.
pub fn default_host_composition_for_grade(grade: DeploymentGrade) -> HostComposition {
    HostComposition::builder()
        .capability_registry(
            everruns_integrations_catalog::oss_capability_registry_for_grade(grade),
        )
        .driver_registry(crate::create_driver_registry())
        // Honor EVERRUNS_SYSTEM_ALLOWLIST_ENABLED for tenant/agent runtime
        // egress (capabilities, MCP, integrations) in distributed workers too.
        .egress_service(Arc::new(DirectEgressService::for_runtime_traffic_from_env()))
        .utility_llm_service(SystemUtilityLlmConfig::from_env().into_service())
        // Guardrail checks on the decisions need the same service in a
        // distributed worker as in the in-process server path.
        .decisions(SystemDecisionsConfig::from_env().into_service())
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A distributed worker runs the same guardrail checks as the in-process
    /// server path, so it has to carry the same decisions. The server's
    /// `oss_composition_carries_a_classifier` asserts this for its side; drift
    /// between the two would make `jev` checks silently no-op under a
    /// distributed deployment while passing every in-process test.
    #[test]
    fn worker_composition_carries_the_same_classifier_as_the_server() {
        let composition = default_host_composition_for_grade(DeploymentGrade::Dev);
        let service = composition.decisions();
        // Process env decides which one; both are valid, a missing service is not.
        let configured =
            std::env::var(everruns_integrations_typesafe::UTILITY_TYPESAFE_API_KEY_ENV)
                .is_ok_and(|key| !key.trim().is_empty());
        assert_eq!(
            service.is_configured(),
            configured,
            "decisions configuration must follow {}",
            everruns_integrations_typesafe::UTILITY_TYPESAFE_API_KEY_ENV
        );
        assert_eq!(
            service.name(),
            if configured {
                "TypeSafeAI"
            } else {
                "DisabledDecisionsService"
            }
        );
    }

    /// The worker advertises the same experimental gating the server does: a
    /// tool the server offered in dev must resolve in the worker that executes
    /// it, and must stay out of a prod worker either way.
    #[test]
    fn jev_capability_follows_the_deployment_grade() {
        assert!(
            default_host_composition_for_grade(DeploymentGrade::Dev)
                .capability_registry()
                .has("jev"),
            "dev workers execute what a dev server offers"
        );
        assert!(
            !default_host_composition_for_grade(DeploymentGrade::Prod)
                .capability_registry()
                .has("jev"),
            "experimental capabilities must stay out of prod registries"
        );
    }
}
