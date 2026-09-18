//! Tests: registry_tests.

use super::*;
use crate::runtime_agent::RuntimeAgent;
use crate::tools::Tool;
use crate::typed_id::SessionId;

use super::tests_support::*;

/// The dispatcher always names the agent being spawned, including when the
/// call carries no usable `target.type` yet.
#[test]
fn unified_spawn_agent_narration_names_the_agent() {
    let tool = UnifiedSpawnAgentTool::new(vec![DelegationTargetProvider {
        target_type: "subagent",
        tool: Box::new(StubSubagentSpawnTool),
    }]);
    let ctx = crate::tool_narration::ToolNarrationContext::default();

    assert_eq!(
        tool.narrate(
            &spawn_agent_call(serde_json::json!({
                "name": "Orbit Scout",
                "target": { "type": "subagent" },
                "blueprint": "github_scout"
            })),
            crate::tool_narration::ToolNarrationPhase::Started,
            None,
            ctx,
        )
        .as_deref(),
        Some("Launching Orbit Scout subagent (github_scout)")
    );

    assert_eq!(
        tool.narrate(
            &spawn_agent_call(serde_json::json!({ "name": "Orbit Scout" })),
            crate::tool_narration::ToolNarrationPhase::Started,
            None,
            ctx,
        )
        .as_deref(),
        Some("Launching Orbit Scout subagent")
    );
}

#[test]
fn unified_spawn_agent_rejects_subagent_fields_for_configured_targets() {
    for target_type in ["agent", "external_a2a"] {
        let arguments = serde_json::json!({
            "target": { "type": target_type, "id": "actual-target" },
            "blueprint": "decoy-target"
        });
        assert_eq!(
            validate_spawn_agent_target_fields(&arguments, target_type),
            Err(format!(
                "blueprint is only valid for subagent targets, not {target_type}."
            ))
        );

        let arguments = serde_json::json!({
            "target": { "type": target_type, "id": "actual-target" },
            "config": { "model": "decoy" }
        });
        assert_eq!(
            validate_spawn_agent_target_fields(&arguments, target_type),
            Err(format!(
                "config is only valid for subagent targets, not {target_type}."
            ))
        );
    }
}

#[test]
fn test_capability_registry_get() {
    let mut registry = CapabilityRegistry::new();
    registry.register(NoopFixture);

    let capability = registry.get("noop").unwrap();
    assert_eq!(capability.id(), "noop");
    assert_eq!(capability.status(), CapabilityStatus::Available);
}

#[test]
fn default_registry_is_empty_and_selects_no_product_preset() {
    assert!(CapabilityRegistry::default().is_empty());
    assert!(CapabilityRegistryBuilder::default().build().is_empty());
}

#[test]
fn blueprint_without_schema_accepts_no_config() {
    let blueprint = blueprint_with_schema(None);

    assert!(blueprint.validate_config(None).is_ok());
    assert!(matches!(
        blueprint.validate_config(Some(&serde_json::json!({"depth": "focused"}))),
        Err(BlueprintConfigError::NotAccepted { .. })
    ));
}

#[test]
fn blueprint_config_is_required_only_when_the_schema_says_so() {
    let required = blueprint_with_schema(Some(
        serde_json::json!({"type": "object", "required": ["repository"]}),
    ));
    assert!(matches!(
        required.validate_config(None),
        Err(BlueprintConfigError::Required { .. })
    ));

    let optional = blueprint_with_schema(Some(serde_json::json!({"type": "object"})));
    assert!(optional.validate_config(None).is_ok());
}

#[test]
fn blueprint_config_is_validated_against_the_schema() {
    let blueprint = blueprint_with_schema(Some(serde_json::json!({
        "type": "object",
        "properties": {
            "max_candidates": {"type": "integer", "minimum": 1, "maximum": 50}
        },
        "additionalProperties": false
    })));

    assert!(
        blueprint
            .validate_config(Some(&serde_json::json!({"max_candidates": 10})))
            .is_ok()
    );

    // Out-of-range values and unknown keys are both contract violations.
    let Err(BlueprintConfigError::Invalid { issues, .. }) =
        blueprint.validate_config(Some(&serde_json::json!({"max_candidates": 500})))
    else {
        panic!("out-of-range config should be rejected");
    };
    assert!(
        issues.iter().any(|issue| issue.contains("max_candidates")),
        "issue should name the offending property: {issues:?}"
    );

    assert!(matches!(
        blueprint.validate_config(Some(&serde_json::json!({"unknown": true}))),
        Err(BlueprintConfigError::Invalid { .. })
    ));
}

#[test]
fn blueprint_with_unusable_schema_reports_it() {
    let blueprint = blueprint_with_schema(Some(serde_json::json!({"type": 42})));

    assert!(matches!(
        blueprint.validate_config(Some(&serde_json::json!({}))),
        Err(BlueprintConfigError::InvalidSchema { .. })
    ));
}

#[tokio::test]
async fn test_capability_registry_blueprint_with_capability() {
    struct BlueprintProviderCapability;

    impl Capability for BlueprintProviderCapability {
        fn id(&self) -> &str {
            "blueprint_provider"
        }
        fn name(&self) -> &str {
            "Blueprint Provider"
        }
        fn description(&self) -> &str {
            "Capability that provides a blueprint for tests"
        }
        fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
            vec![AgentBlueprint {
                id: "test_blueprint",
                name: "Test Blueprint",
                description: "Blueprint for capability registry tests",
                model: BlueprintModel::Fixed("specialist-model".into()),
                system_prompt: "Test prompt",
                tools: vec![Box::new(FixtureTool("private_lookup"))],
                max_turns: Some(7),
                config_schema: Some(
                    serde_json::json!({"type":"object", "required":["repository"]}),
                ),
            }]
        }
    }

    let mut registry = CapabilityRegistry::new();
    registry.register(BlueprintProviderCapability);

    let (capability_id, blueprint) = registry
        .blueprint_with_capability("test_blueprint")
        .expect("blueprint should resolve with capability id");
    assert_eq!(capability_id, "blueprint_provider");
    assert_eq!(blueprint.id, "test_blueprint");
    assert_eq!(blueprint.name, "Test Blueprint");
    assert_eq!(
        blueprint.description,
        "Blueprint for capability registry tests"
    );
    assert_eq!(blueprint.system_prompt, "Test prompt");
    assert!(
        matches!(&blueprint.model, BlueprintModel::Fixed(model) if model == "specialist-model")
    );
    assert_eq!(blueprint.max_turns, Some(7));
    assert_eq!(
        blueprint.config_schema,
        Some(serde_json::json!({"type":"object", "required":["repository"]}))
    );
    let definitions = blueprint.tool_definitions();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].name(), "private_lookup");
    assert_eq!(
        registry.blueprint("test_blueprint").unwrap().tools[0].name(),
        "private_lookup"
    );
    assert_eq!(
        registry
            .all_blueprints()
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>(),
        ["test_blueprint"]
    );
    assert!(registry.blueprint_with_capability("missing").is_none());
    assert!(registry.blueprint("missing").is_none());
    let host = collect_capabilities(&["blueprint_provider".into()], &registry, &test_ctx()).await;
    assert!(host.tools.is_empty());
    assert!(host.tool_definitions.is_empty());
}

#[test]
fn test_capability_registry_builder() {
    let registry = CapabilityRegistry::builder()
        .capability(NoopFixture)
        .build();

    assert!(registry.has("noop"));
    assert_eq!(registry.len(), 1);
}

#[test]
fn test_system_prompt_preview_default_delegates_to_addition() {
    // A capability with a static system_prompt_addition — preview should
    // match the addition by default.
    struct StaticPromptCapability;
    impl Capability for StaticPromptCapability {
        fn id(&self) -> &str {
            "static_prompt"
        }
        fn name(&self) -> &str {
            "Static Prompt"
        }
        fn description(&self) -> &str {
            "Static prompt addition."
        }
        fn system_prompt_addition(&self) -> Option<&str> {
            Some("Use the static prompt.")
        }
    }

    let cap = StaticPromptCapability;
    assert_eq!(
        cap.system_prompt_preview().as_deref(),
        Some("Use the static prompt.")
    );

    // current_time has no system_prompt_addition — preview should be None
    let registry = fixture_registry();
    let current_time = registry.get("current_time").unwrap();
    assert!(current_time.system_prompt_preview().is_none());
    assert!(current_time.system_prompt_addition().is_none());
}

// =========================================================================
// apply_capabilities tests
// =========================================================================

#[tokio::test]
async fn test_apply_capabilities_empty() {
    let registry = CapabilityRegistry::new();
    let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

    let applied = apply_capabilities(base_runtime_agent.clone(), &[], &registry, &test_ctx()).await;

    assert_eq!(
        applied.runtime_agent.system_prompt,
        base_runtime_agent.system_prompt
    );
    assert!(applied.tool_registry.is_empty());
    assert!(applied.applied_ids.is_empty());
}

#[tokio::test]
async fn test_apply_capabilities_noop() {
    let registry = fixture_registry();
    let mut base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

    base_runtime_agent.max_iterations = 13;
    base_runtime_agent.temperature = Some(0.25);
    base_runtime_agent.max_tokens = Some(1234);
    base_runtime_agent.parallel_tool_calls = Some(false);
    let applied = apply_capabilities(
        base_runtime_agent.clone(),
        &["noop".to_string()],
        &registry,
        &test_ctx(),
    )
    .await;

    // Noop has no system prompt addition or tools
    assert_eq!(
        applied.runtime_agent.system_prompt,
        base_runtime_agent.system_prompt
    );
    assert!(applied.tool_registry.is_empty());
    assert_eq!(applied.applied_ids, vec!["noop"]);
    assert_eq!(
        serde_json::to_value(&applied.runtime_agent).unwrap(),
        serde_json::to_value(&base_runtime_agent).unwrap()
    );
    let collected = collect_capabilities(&["noop".into()], &registry, &test_ctx()).await;
    assert!(collected.mounts.is_empty());
    assert!(collected.message_filter_providers.is_empty());
    assert!(compute_features(&["noop".into()], &registry).is_empty());
}

#[tokio::test]
async fn test_apply_capabilities_current_time() {
    let registry = fixture_registry();
    let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

    let applied = apply_capabilities(
        base_runtime_agent.clone(),
        &["current_time".to_string()],
        &registry,
        &test_ctx(),
    )
    .await;

    // CurrentTime contributes a dynamic `current_time` fact, so the cached
    // prompt gains the explanatory facts note (the live value is appended at
    // the conversation tail per request). It also keeps its tool.
    assert!(
        applied
            .runtime_agent
            .system_prompt
            .contains(FACTS_DYNAMIC_NOTE),
        "current_time should contribute the dynamic-facts note"
    );
    assert!(
        applied
            .runtime_agent
            .system_prompt
            .contains(&base_runtime_agent.system_prompt),
        "base prompt is preserved"
    );
    assert!(applied.tool_registry.has("get_current_time"));
    assert_eq!(applied.tool_registry.len(), 1);
    assert_eq!(applied.applied_ids, vec!["current_time"]);
}

#[tokio::test]
async fn test_apply_capabilities_skips_coming_soon() {
    struct ComingSoonFixture;
    impl Capability for ComingSoonFixture {
        fn id(&self) -> &str {
            "coming_soon_fixture"
        }
        fn name(&self) -> &str {
            "Coming Soon Fixture"
        }
        fn description(&self) -> &str {
            "Test-only capability."
        }
        fn status(&self) -> CapabilityStatus {
            CapabilityStatus::ComingSoon
        }
        fn system_prompt_addition(&self) -> Option<&str> {
            Some("Not yet available.")
        }
    }
    let mut registry = CapabilityRegistry::new();
    registry.register(ComingSoonFixture);
    let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

    let applied = apply_capabilities(
        base_runtime_agent.clone(),
        &["coming_soon_fixture".to_string()],
        &registry,
        &test_ctx(),
    )
    .await;

    assert_eq!(
        applied.runtime_agent.system_prompt,
        base_runtime_agent.system_prompt
    );
    assert!(applied.applied_ids.is_empty());
}

/// A deprecated capability has only *announced* its removal, so it must keep
/// behaving exactly as before. This is the regression guard for the gating
/// switch from `status() == Available` to `status().is_active()`.
#[tokio::test]
async fn test_apply_capabilities_keeps_deprecated_fully_functional() {
    struct DeprecatedFixture;
    impl Capability for DeprecatedFixture {
        fn id(&self) -> &str {
            "deprecated_fixture"
        }
        fn name(&self) -> &str {
            "Deprecated Fixture"
        }
        fn description(&self) -> &str {
            "Test-only capability."
        }
        fn status(&self) -> CapabilityStatus {
            CapabilityStatus::Deprecated
        }
        fn system_prompt_addition(&self) -> Option<&str> {
            Some("Still working.")
        }
    }
    let mut registry = CapabilityRegistry::new();
    registry.register(DeprecatedFixture);
    let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

    let applied = apply_capabilities(
        base_runtime_agent,
        &["deprecated_fixture".to_string()],
        &registry,
        &test_ctx(),
    )
    .await;

    assert!(
        applied
            .runtime_agent
            .system_prompt
            .contains("Still working.")
    );
    assert_eq!(applied.applied_ids, vec!["deprecated_fixture"]);
}

/// A retired capability is inert, but an agent that still references one must
/// keep running: the reference resolves to a no-op and every other capability
/// in the list still applies.
#[tokio::test]
async fn test_apply_capabilities_skips_retired_without_failing() {
    struct RetiredFixture;
    impl Capability for RetiredFixture {
        fn id(&self) -> &str {
            "retired_fixture"
        }
        fn name(&self) -> &str {
            "Retired Fixture"
        }
        fn description(&self) -> &str {
            "Test-only capability."
        }
        fn status(&self) -> CapabilityStatus {
            CapabilityStatus::Retired
        }
        fn system_prompt_addition(&self) -> Option<&str> {
            Some("Should never be applied.")
        }
    }
    let mut registry = fixture_registry();
    registry.register(RetiredFixture);
    let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

    let applied = apply_capabilities(
        base_runtime_agent,
        &["retired_fixture".to_string(), "current_time".to_string()],
        &registry,
        &test_ctx(),
    )
    .await;

    assert!(
        !applied
            .runtime_agent
            .system_prompt
            .contains("Should never be applied.")
    );
    // The surviving capability in the same list is unaffected.
    assert_eq!(applied.applied_ids, vec!["current_time"]);
    assert!(applied.tool_registry.has("get_current_time"));
}

#[tokio::test]
async fn test_apply_capabilities_preserves_order() {
    let registry = fixture_registry();
    let base_runtime_agent = RuntimeAgent::new("Base prompt.", "gpt-5.2");

    // Order should be preserved in applied_ids
    let applied = apply_capabilities(
        base_runtime_agent,
        &["current_time".to_string(), "noop".to_string()],
        &registry,
        &test_ctx(),
    )
    .await;

    assert_eq!(applied.applied_ids, vec!["current_time", "noop"]);
    assert_eq!(applied.tool_registry.len(), 1);
    assert!(applied.tool_registry.has("get_current_time"));
}

// =========================================================================
// XML prompt formatting tests
// =========================================================================

// =========================================================================
// Mount collection tests
// =========================================================================

#[tokio::test]
async fn test_dynamic_facts_add_note_without_static_block() {
    // `current_time` contributes a Dynamic fact, so the cached prompt gets
    // the explanatory note but NOT a static `<facts>` block (the live value
    // is appended at the conversation tail per request instead).
    let registry = fixture_registry();
    let configs = vec![AgentCapabilityConfig::new("current_time".to_string())];
    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
    let prompt = collected.system_prompt_parts.join("\n");
    assert!(
        prompt.contains(FACTS_DYNAMIC_NOTE),
        "dynamic-facts note should be in the cached prompt"
    );
    assert!(
        !prompt.contains("<facts>\n"),
        "no static <facts> block for a purely-dynamic fact; got: {prompt}"
    );
}

#[tokio::test]
async fn test_static_facts_fold_into_prompt() {
    struct StaticFactCap;
    impl Capability for StaticFactCap {
        fn id(&self) -> &str {
            "test_static_fact"
        }
        fn name(&self) -> &str {
            "Static Fact"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn status(&self) -> CapabilityStatus {
            CapabilityStatus::Available
        }
        fn facts(&self, _config: &serde_json::Value, _ctx: &FactsContext) -> Vec<Fact> {
            vec![Fact::stat("workspace_root", "/workspace")]
        }
    }
    let mut registry = CapabilityRegistry::new();
    registry.register(StaticFactCap);
    let configs = vec![AgentCapabilityConfig::new("test_static_fact".to_string())];
    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
    let prompt = collected.system_prompt_parts.join("\n");
    assert!(
        prompt.contains("<facts>\n- workspace_root: /workspace\n</facts>"),
        "static fact should fold into the cached prompt; got: {prompt}"
    );
    assert!(
        !prompt.contains(FACTS_DYNAMIC_NOTE),
        "no dynamic note when only static facts exist"
    );
}

#[test]
fn test_collect_dynamic_facts_returns_current_time() {
    let registry = fixture_registry();
    let configs = vec![AgentCapabilityConfig::new("current_time".to_string())];
    let facts = collect_dynamic_facts(
        &configs,
        &registry,
        None,
        &FactsContext::new(SessionId::new()),
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].key, "current_time");
    assert_eq!(facts[0].value, "fixture-now");
    assert_eq!(facts[0].volatility, Volatility::Dynamic);
}

#[tokio::test]
async fn test_collect_capabilities_combines_mounts() {
    struct Notes;
    impl Capability for Notes {
        fn id(&self) -> &str {
            "notes"
        }
        fn name(&self) -> &str {
            "Notes"
        }
        fn description(&self) -> &str {
            "Writable notes"
        }
        fn mounts(&self) -> Vec<MountPoint> {
            vec![MountPoint::readwrite(
                "/notes.txt",
                MountSource::text_file("Note α"),
                "notes",
            )]
        }
    }
    let mut registry = fixture_registry();
    registry.register(Notes);
    let collected = collect_capabilities(
        &["sample_data".into(), "notes".into(), "current_time".into()],
        &registry,
        &test_ctx(),
    )
    .await;
    assert_eq!(
        collected.applied_ids,
        [
            "session_file_system",
            "sample_data",
            "notes",
            "current_time"
        ]
    );
    assert_eq!(
        collected.mounts,
        vec![
            MountPoint::readonly(
                "/samples",
                MountDirectoryBuilder::new()
                    .file("users.json", "[]")
                    .build(),
                "sample_data"
            ),
            MountPoint::readwrite("/notes.txt", MountSource::text_file("Note α"), "notes"),
        ]
    );
}

// =========================================================================
// Dependency resolution tests
// =========================================================================

#[test]
fn test_resolve_dependencies_empty() {
    let registry = CapabilityRegistry::new();

    let resolved = resolve_dependencies(&[], &registry).unwrap();

    assert!(resolved.resolved_ids.is_empty());
    assert!(resolved.added_as_dependencies.is_empty());
    assert!(resolved.user_selected.is_empty());
}

#[test]
fn test_resolve_dependencies_no_deps() {
    let registry = fixture_registry();

    // CurrentTime has no dependencies
    let resolved = resolve_dependencies(&["current_time".to_string()], &registry).unwrap();

    assert_eq!(resolved.resolved_ids, vec!["current_time"]);
    assert!(resolved.added_as_dependencies.is_empty());
}

#[test]
fn test_resolve_dependencies_with_deps() {
    let resolved = resolve_dependencies(&["sample_data".into()], &fixture_registry()).unwrap();
    assert_eq!(
        resolved.resolved_ids,
        ["session_file_system", "sample_data"]
    );
    assert_eq!(resolved.added_as_dependencies, ["session_file_system"]);
    assert_eq!(resolved.user_selected, ["sample_data"]);
}

#[test]
fn test_resolve_dependencies_already_selected() {
    let registry = fixture_registry();

    // If dependency is already selected, it shouldn't be duplicated
    let resolved = resolve_dependencies(
        &["session_file_system".to_string(), "sample_data".to_string()],
        &registry,
    )
    .unwrap();

    assert_eq!(resolved.resolved_ids.len(), 2);
    // FileSystem was user-selected, not added as dependency
    assert!(resolved.added_as_dependencies.is_empty());
}

#[test]
fn test_resolve_dependencies_preserves_order() {
    let registry = fixture_registry();

    // Multiple independent capabilities should maintain their relative order
    let resolved =
        resolve_dependencies(&["current_time".to_string(), "noop".to_string()], &registry).unwrap();

    assert_eq!(resolved.resolved_ids, vec!["current_time", "noop"]);
}

#[test]
fn test_resolve_dependencies_unknown_capability() {
    let registry = CapabilityRegistry::new();

    // Unknown capabilities are silently skipped
    let resolved = resolve_dependencies(&["unknown_capability".to_string()], &registry).unwrap();

    assert!(resolved.resolved_ids.is_empty());
}

#[test]
fn test_get_dependencies() {
    let registry = fixture_registry();

    // SampleData depends on FileSystem
    let deps = get_dependencies("sample_data", &registry);
    assert_eq!(deps, vec!["session_file_system"]);

    // CurrentTime has no dependencies
    let deps = get_dependencies("current_time", &registry);
    assert!(deps.is_empty());

    // Unknown capability
    let deps = get_dependencies("unknown", &registry);
    assert!(deps.is_empty());
}

// Test for circular dependency detection
// Note: We can't easily test this with built-in capabilities since they don't have cycles.
// This test uses a custom registry to create a cycle.
#[test]
fn test_circular_dependency_error() {
    // Create capabilities that form a cycle: A -> B -> A
    struct CapA;
    struct CapB;

    impl Capability for CapA {
        fn id(&self) -> &str {
            "test_cap_a"
        }
        fn name(&self) -> &str {
            "Test A"
        }
        fn description(&self) -> &str {
            "Test capability A"
        }
        fn dependencies(&self) -> Vec<&'static str> {
            vec!["test_cap_b"]
        }
    }

    impl Capability for CapB {
        fn id(&self) -> &str {
            "test_cap_b"
        }
        fn name(&self) -> &str {
            "Test B"
        }
        fn description(&self) -> &str {
            "Test capability B"
        }
        fn dependencies(&self) -> Vec<&'static str> {
            vec!["test_cap_a"]
        }
    }

    let mut registry = CapabilityRegistry::new();
    registry.register(CapA);
    registry.register(CapB);

    let result = resolve_dependencies(&["test_cap_a".to_string()], &registry);

    assert!(result.is_err());
    match result.unwrap_err() {
        DependencyError::CircularDependency { capability_id, .. } => {
            assert_eq!(capability_id, "test_cap_a");
        }
        _ => panic!("Expected CircularDependency error"),
    }
}

// =========================================================================
// Message filter provider tests
// =========================================================================

#[tokio::test]
async fn test_collect_capabilities_with_configs_no_filter_providers() {
    let registry = fixture_registry();
    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("current_time"),
        serde_json::json!({}),
    )];

    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

    assert!(collected.message_filter_providers.is_empty());
    assert!(!collected.has_message_filters());
}
