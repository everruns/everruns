//! Worker runtime platform helpers.
//!
//! Workers honor the same `PlatformDefinition` shape as the server so
//! embedders can keep execution and control-plane runtime surfaces aligned.
//! The worker default only includes capabilities and LLM drivers because
//! connection providers and harness templates are server-owned by default.

use everruns_core::{
    CapabilityRegistry, DeploymentGrade, PlatformDefinition, SystemUtilityLlmConfig,
};

/// Build the default worker-side platform definition for the current deployment grade.
pub fn default_platform_definition() -> PlatformDefinition {
    default_platform_definition_for_grade(DeploymentGrade::from_env())
}

/// Build the default worker-side platform definition for an explicit grade.
pub fn default_platform_definition_for_grade(grade: DeploymentGrade) -> PlatformDefinition {
    PlatformDefinition::builder()
        .capability_registry(CapabilityRegistry::with_builtins_for_grade(grade))
        .driver_registry(crate::create_driver_registry())
        .utility_llm_service(SystemUtilityLlmConfig::from_env().into_service())
        .build()
}
