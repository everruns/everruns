//! Tests: collect_tests.

use super::*;
use crate::message::Message;
use crate::message_filter::{MessageFilter, MessageFilterProvider, MessageQuery};
use crate::runtime_agent::RuntimeAgent;
use crate::tool_types::ToolDefinition;
use crate::typed_id::SessionId;
use std::sync::Arc;
use uuid::Uuid;

use super::tests_support::*;

#[tokio::test]
async fn test_collected_capabilities_apply_message_filters() {
    let mut registry = CapabilityRegistry::new();
    registry.register(FilterTestCapability { priority: 0 });

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("filter_test"),
        serde_json::json!({ "search": "test_query" }),
    )];

    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

    assert!(collected.has_message_filters());

    // Apply filters to a query
    let session_id: SessionId = Uuid::now_v7().into();
    let mut query = MessageQuery::new(session_id);

    collected.apply_message_filters(&mut query);

    // Should have added the search filter
    assert_eq!(query.filters.len(), 1);
    assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "test_query"));
}

#[tokio::test]
async fn test_collected_capabilities_apply_multiple_filters_in_priority_order() {
    struct SearchCapability {
        id: &'static str,
        search_term: &'static str,
        priority: i32,
    }

    struct SearchProvider {
        search_term: &'static str,
        priority: i32,
    }

    impl MessageFilterProvider for SearchProvider {
        fn apply_filters(&self, query: &mut MessageQuery, _config: &serde_json::Value) {
            query
                .filters
                .push(MessageFilter::Search(self.search_term.to_string()));
        }

        fn priority(&self) -> i32 {
            self.priority
        }
    }

    impl Capability for SearchCapability {
        fn id(&self) -> &str {
            self.id
        }
        fn name(&self) -> &str {
            "Search"
        }
        fn description(&self) -> &str {
            "Test"
        }
        fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
            Some(Arc::new(SearchProvider {
                search_term: self.search_term,
                priority: self.priority,
            }))
        }
    }

    let mut registry = CapabilityRegistry::new();
    registry.register(SearchCapability {
        id: "cap_a",
        search_term: "alpha",
        priority: 5,
    });
    registry.register(SearchCapability {
        id: "cap_b",
        search_term: "beta",
        priority: 1,
    });
    registry.register(SearchCapability {
        id: "cap_c",
        search_term: "gamma",
        priority: 10,
    });

    let configs = vec![
        AgentCapabilityConfig::with_config(CapabilityId::new("cap_a"), serde_json::json!({})),
        AgentCapabilityConfig::with_config(CapabilityId::new("cap_b"), serde_json::json!({})),
        AgentCapabilityConfig::with_config(CapabilityId::new("cap_c"), serde_json::json!({})),
    ];

    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

    let session_id: SessionId = Uuid::now_v7().into();
    let mut query = MessageQuery::new(session_id);

    collected.apply_message_filters(&mut query);

    // Filters should be applied in priority order: beta (1), alpha (5), gamma (10)
    assert_eq!(query.filters.len(), 3);
    assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "beta"));
    assert!(matches!(&query.filters[1], MessageFilter::Search(s) if s == "alpha"));
    assert!(matches!(&query.filters[2], MessageFilter::Search(s) if s == "gamma"));
}

#[tokio::test]
async fn test_collect_capabilities_preserves_config_for_filter_provider() {
    let mut registry = CapabilityRegistry::new();
    registry.register(FilterTestCapability { priority: 0 });

    let test_config = serde_json::json!({
        "search": "custom_search",
        "extra_field": 42
    });

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("filter_test"),
        test_config.clone(),
    )];

    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

    // Verify the config is preserved
    assert_eq!(collected.message_filter_providers.len(), 1);
    let (_, stored_config) = &collected.message_filter_providers[0];
    assert_eq!(*stored_config, test_config);
}

// =========================================================================
// collect_message_filters_only tests
// =========================================================================

#[test]
fn test_collect_message_filters_only_collects_filters() {
    let mut registry = CapabilityRegistry::new();
    registry.register(FilterTestCapability { priority: 0 });

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("filter_test"),
        serde_json::json!({ "search": "test_query" }),
    )];

    let collected = collect_message_filters_only(&configs, &registry);

    let session_id: SessionId = Uuid::now_v7().into();
    let mut query = MessageQuery::new(session_id);
    collected.apply_message_filters(&mut query);

    assert_eq!(query.filters.len(), 1);
    assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "test_query"));
}

#[test]
fn test_collect_message_filters_only_skips_unknown_capabilities() {
    let registry = CapabilityRegistry::new();

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("nonexistent"),
        serde_json::json!({}),
    )];

    let collected = collect_message_filters_only(&configs, &registry);
    assert!(collected.message_filter_providers.is_empty());
}

#[test]
fn test_collect_message_filters_only_preserves_priority_order() {
    struct PriorityFilterCap {
        id: &'static str,
        search_term: &'static str,
        priority: i32,
    }

    struct PriorityFilterProvider {
        search_term: &'static str,
        priority: i32,
    }

    impl Capability for PriorityFilterCap {
        fn id(&self) -> &str {
            self.id
        }
        fn name(&self) -> &str {
            self.id
        }
        fn description(&self) -> &str {
            "priority test"
        }
        fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
            Some(Arc::new(PriorityFilterProvider {
                search_term: self.search_term,
                priority: self.priority,
            }))
        }
    }

    impl MessageFilterProvider for PriorityFilterProvider {
        fn apply_filters(&self, query: &mut MessageQuery, _config: &serde_json::Value) {
            query
                .filters
                .push(MessageFilter::Search(self.search_term.to_string()));
        }
        fn priority(&self) -> i32 {
            self.priority
        }
    }

    let mut registry = CapabilityRegistry::new();
    registry.register(PriorityFilterCap {
        id: "gamma",
        search_term: "gamma",
        priority: 10,
    });
    registry.register(PriorityFilterCap {
        id: "alpha",
        search_term: "alpha",
        priority: 5,
    });
    registry.register(PriorityFilterCap {
        id: "beta",
        search_term: "beta",
        priority: 1,
    });

    let configs = vec![
        AgentCapabilityConfig::with_config(CapabilityId::new("gamma"), serde_json::json!({})),
        AgentCapabilityConfig::with_config(CapabilityId::new("alpha"), serde_json::json!({})),
        AgentCapabilityConfig::with_config(CapabilityId::new("beta"), serde_json::json!({})),
    ];

    let collected = collect_message_filters_only(&configs, &registry);

    let session_id: SessionId = Uuid::now_v7().into();
    let mut query = MessageQuery::new(session_id);
    collected.apply_message_filters(&mut query);

    // Filters should be applied in priority order: beta (1), alpha (5), gamma (10)
    assert_eq!(query.filters.len(), 3);
    assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "beta"));
    assert!(matches!(&query.filters[1], MessageFilter::Search(s) if s == "alpha"));
    assert!(matches!(&query.filters[2], MessageFilter::Search(s) if s == "gamma"));
}

#[test]
fn test_collect_message_filters_only_post_load_invoked() {
    use crate::message::Message;

    struct PostLoadCap;
    struct PostLoadProvider;

    impl Capability for PostLoadCap {
        fn id(&self) -> &str {
            "post_load_test"
        }
        fn name(&self) -> &str {
            "PostLoad Test"
        }
        fn description(&self) -> &str {
            "test"
        }
        fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
            Some(Arc::new(PostLoadProvider))
        }
    }

    impl MessageFilterProvider for PostLoadProvider {
        fn apply_filters(&self, _query: &mut MessageQuery, _config: &serde_json::Value) {}
        fn priority(&self) -> i32 {
            0
        }
        fn post_load(&self, messages: &mut Vec<Message>, _config: &serde_json::Value) {
            // Reverse messages to prove post_load was called
            messages.reverse();
        }
    }

    let mut registry = CapabilityRegistry::new();
    registry.register(PostLoadCap);

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("post_load_test"),
        serde_json::json!({}),
    )];

    let collected = collect_message_filters_only(&configs, &registry);

    let mut messages = vec![Message::user("first"), Message::user("second")];
    collected.apply_post_load_filters(&mut messages);

    // post_load reversed the messages
    assert_eq!(messages[0].text(), Some("second"));
    assert_eq!(messages[1].text(), Some("first"));
}

// Tests for resolve_for_model delegation in fast-path collectors

#[test]
fn test_collect_message_filters_only_honors_resolve_for_model_delegation() {
    let inner = std::sync::Arc::new(InnerFilterCap);
    let outer = DelegatingFilterCap {
        id: "delegating_filter",
        inner: inner.clone(),
    };

    let mut registry = CapabilityRegistry::new();
    registry.register(outer);

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("delegating_filter"),
        serde_json::json!({"limit": 17}),
    )];

    // Outer has no message_filter_provider; inner does. resolve_for_model
    // delegates to inner so the provider should be collected.
    let collected = collect_message_filters_only(&configs, &registry);
    assert_eq!(
        collected.message_filter_providers.len(),
        1,
        "provider from resolved inner capability must be collected"
    );
    let mut query = MessageQuery::default();
    collected.apply_message_filters(&mut query);
    assert_eq!(query.limit, Some(17));
}

#[test]
fn test_collect_model_view_providers_honors_resolve_for_model_delegation() {
    let inner = std::sync::Arc::new(InnerMvpCap);
    let outer = DelegatingMvpCap {
        id: "delegating_mvp",
        inner: inner.clone(),
    };

    let mut registry = CapabilityRegistry::new();
    registry.register(outer);

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("delegating_mvp"),
        serde_json::json!({"suffix": "delegated"}),
    )];

    // Outer has no model_view_provider; inner does. resolve_for_model
    // delegates to inner so the provider should be collected.
    let collected = collect_model_view_providers(&configs, &registry, Some("selected-model"));
    assert_eq!(
        collected.model_view_providers.len(),
        1,
        "provider from resolved inner capability must be collected"
    );
    assert!(
        collect_model_view_providers(&configs, &registry, Some("other-model"))
            .model_view_providers
            .is_empty()
    );
    let session_id = SessionId::from_seed(42);
    let output = collected.apply_model_view(
        vec![Message::user("original")],
        &ModelViewContext {
            session_id,
            prior_usage: None,
        },
    );
    assert_eq!(
        output.iter().map(Message::text).collect::<Vec<_>>(),
        [
            Some("original"),
            Some(format!("delegated:{session_id}").as_str())
        ]
    );
}

// =========================================================================
// Default selection and alias resolution
// =========================================================================

#[test]
fn test_defaults_do_not_include_bash() {
    // ToolRegistry::with_defaults() must NOT include bash — it comes from
    // capabilities only. This documents the invariant that the bug violated.
    let registry = crate::ToolRegistry::with_defaults();
    assert!(
        !registry.has("bash"),
        "with_defaults() must not include 'bash' — it comes from bashkit_shell capability"
    );
}

// =========================================================================
// Feature tests
// =========================================================================

#[test]
fn test_alias_resolves_to_canonical_capability() {
    let registry = fixture_registry();

    // Legacy `virtual_bash` ID (persisted agent configs) must keep working.
    let via_alias = registry.get("virtual_bash").unwrap();
    assert_eq!(via_alias.id(), "bashkit_shell");
    assert!(registry.has("virtual_bash"));
    assert_eq!(registry.canonical_id("virtual_bash"), Some("bashkit_shell"));
    assert_eq!(
        registry.canonical_id("bashkit_shell"),
        Some("bashkit_shell")
    );
    assert_eq!(registry.canonical_id("nonexistent"), None);
}

#[test]
fn test_alias_dedupes_with_canonical_in_dependency_resolution() {
    let registry = fixture_registry();

    // Selecting both the alias and the canonical ID must resolve to a
    // single activation under the canonical ID.
    let resolved = resolve_dependencies(
        &["virtual_bash".to_string(), "bashkit_shell".to_string()],
        &registry,
    )
    .unwrap();
    let bash_ids: Vec<_> = resolved
        .resolved_ids
        .iter()
        .filter(|id| id.as_str() == "bashkit_shell" || id.as_str() == "virtual_bash")
        .collect();
    assert_eq!(bash_ids, vec!["bashkit_shell"]);
    // Selected via alias => not reported as "added as dependency".
    assert!(
        !resolved
            .added_as_dependencies
            .contains(&"bashkit_shell".to_string())
    );
}

#[test]
fn test_alias_preserves_explicit_config_in_resolution() {
    let registry = fixture_registry();

    let configs = vec![AgentCapabilityConfig::with_config(
        "virtual_bash".to_string(),
        serde_json::json!({"key": "value"}),
    )];
    let resolved = resolve_capability_configs(&configs, &registry).unwrap();
    let bash = resolved
        .iter()
        .find(|c| c.capability_id() == "bashkit_shell")
        .expect("alias must resolve to canonical bashkit_shell config");
    assert_eq!(
        bash.config_value().clone(),
        serde_json::json!({"key": "value"})
    );
}

#[test]
fn test_unregister_by_alias_removes_capability_and_aliases() {
    let mut registry = fixture_registry();

    assert!(registry.unregister("virtual_bash").is_some());
    assert!(!registry.has("bashkit_shell"));
    assert!(!registry.has("virtual_bash"));
}

#[test]
fn test_compute_features_empty() {
    let registry = CapabilityRegistry::new();

    let features = compute_features(&[], &registry);
    assert!(features.is_empty());
}

#[test]
fn test_compute_features_unknown_capability_ignored() {
    let registry = fixture_registry();

    let features = compute_features(
        &["unknown_cap".to_string(), "session_storage".to_string()],
        &registry,
    );
    assert_eq!(features, vec!["secrets", "key_value"]);
}

#[test]
fn test_risk_level_ordering() {
    assert!(RiskLevel::Low < RiskLevel::Medium);
    assert!(RiskLevel::Medium < RiskLevel::High);
}

#[test]
fn test_risk_level_serde_roundtrip() {
    for (level, wire) in [
        (RiskLevel::Low, "\"low\""),
        (RiskLevel::Medium, "\"medium\""),
        (RiskLevel::High, "\"high\""),
    ] {
        assert_eq!(serde_json::to_string(&level).unwrap(), wire);
        assert_eq!(serde_json::from_str::<RiskLevel>(wire).unwrap(), level);
    }
    assert!(serde_json::from_str::<RiskLevel>("\"critical\"").is_err());
}

// ========================================================================
// contribute_skills() collection — EVE-311
// ========================================================================

#[tokio::test]
async fn test_contribute_skills_normalized_to_mounts() {
    let mut registry = CapabilityRegistry::new();
    registry.register(SkillContributingCapability);

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("contributes_skills"),
        serde_json::json!({}),
    )];

    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

    let skill_mounts: Vec<_> = collected
        .mounts
        .iter()
        .filter(|m| m.path.starts_with("/.agents/skills/"))
        .collect();
    assert_eq!(skill_mounts.len(), 2);

    // Every contributed skill mount is read-only and owned by the contributing
    // capability so the VFS layer can attribute skill files correctly.
    for m in &skill_mounts {
        assert!(m.is_readonly());
        assert_eq!(m.capability_id, "contributes_skills");
    }

    let alpha = skill_mounts
        .iter()
        .find(|m| m.path == "/.agents/skills/alpha-skill")
        .expect("alpha-skill mount missing");
    match &alpha.source {
        MountSource::InlineDirectory { entries } => {
            assert!(entries.contains_key("SKILL.md"));
            assert!(entries.contains_key("scripts/a.sh"));
            let parsed = crate::skill::parse_skill_md(skill_md_from_entries(entries)).unwrap();
            assert_eq!(parsed.name, "alpha-skill");
            assert_eq!(parsed.description, "Alpha skill desc");
            assert_eq!(parsed.instructions, "# Alpha\nDo alpha.");
            assert!(parsed.user_invocable);
        }
        _ => panic!("Expected InlineDirectory"),
    }

    let beta = skill_mounts
        .iter()
        .find(|m| m.path == "/.agents/skills/beta-skill")
        .expect("beta-skill mount missing");
    match &beta.source {
        MountSource::InlineDirectory { entries } => {
            let parsed = crate::skill::parse_skill_md(skill_md_from_entries(entries)).unwrap();
            assert!(!parsed.user_invocable);
            assert_eq!(parsed.name, "beta-skill");
            assert_eq!(parsed.instructions, "# Beta\nDo beta.");
        }
        _ => panic!("Expected InlineDirectory"),
    }
}

#[tokio::test]
async fn test_contribute_skills_default_empty() {
    // Registry-resident capability without a contribute_skills override
    // must not add skill mounts.
    let mut registry = CapabilityRegistry::new();
    registry.register(FilterTestCapability { priority: 0 });

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("filter_test"),
        serde_json::json!({}),
    )];

    let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
    assert!(
        collected
            .mounts
            .iter()
            .all(|m| !m.path.starts_with("/.agents/skills/"))
    );
}

#[test]
fn localized_name_falls_back_exact_language_then_base() {
    let cap = LocalizedCapability;
    // Exact region wins; an absent regional field still falls back by language.
    assert_eq!(cap.localized_name(Some("uk-UA")), "Регіональна");
    assert_eq!(cap.localized_name(Some("uk")), "Локалізована");
    assert_eq!(cap.localized_name(Some("uk-CA")), "Локалізована");
    assert_eq!(cap.localized_name(Some(" UK_ua ")), "Регіональна");
    assert_eq!(cap.localized_description(Some("uk-UA")), "Український опис");
    // Underscore-separated tags are normalized.
    assert_eq!(cap.localized_name(Some("uk_UA")), "Регіональна");
    // Unsupported locales and None fall back to the base name.
    assert_eq!(cap.localized_name(Some("fr-FR")), "Localized");
    assert_eq!(cap.localized_name(None), "Localized");
    assert_eq!(cap.localized_description(Some("uk")), "Український опис");
    assert_eq!(cap.localized_description(Some("de")), "English description");
}

#[test]
fn describe_schema_resolves_config_description_per_locale() {
    let cap = LocalizedCapability;
    assert_eq!(
        cap.describe_schema(Some("uk-UA")).as_deref(),
        Some("Керує налаштуваннями.")
    );
    // Unsupported locales fall back to the "en" entry.
    assert_eq!(
        cap.describe_schema(Some("pl")).as_deref(),
        Some("Controls things.")
    );
    assert_eq!(
        cap.describe_schema(None).as_deref(),
        Some("Controls things.")
    );
    // Capabilities without localizations have no config description.
    assert_eq!(HostAnnotatedCapability.describe_schema(Some("uk")), None);
}

#[tokio::test]
async fn collection_preserves_exact_tool_identity_schema_and_attribution() {
    let registry = fixture_registry();
    for (ids, expected) in [
        (
            vec!["test_math"],
            vec![
                ("add", "test_math", "Test Math"),
                ("subtract", "test_math", "Test Math"),
                ("multiply", "test_math", "Test Math"),
                ("divide", "test_math", "Test Math"),
            ],
        ),
        (
            vec!["test_weather"],
            vec![
                ("get_weather", "test_weather", "Test Weather"),
                ("get_forecast", "test_weather", "Test Weather"),
            ],
        ),
        (
            vec!["sample_data"],
            vec![
                ("read_file", "session_file_system", "Fixture Filesystem"),
                ("write_file", "session_file_system", "Fixture Filesystem"),
            ],
        ),
        (
            vec!["bashkit_shell", "test_weather"],
            vec![
                ("read_file", "session_file_system", "Fixture Filesystem"),
                ("write_file", "session_file_system", "Fixture Filesystem"),
                ("bash", "bashkit_shell", "Fixture Bash"),
                ("get_weather", "test_weather", "Test Weather"),
                ("get_forecast", "test_weather", "Test Weather"),
            ],
        ),
    ] {
        let ids: Vec<_> = ids.into_iter().map(String::from).collect();
        let collected = collect_capabilities(&ids, &registry, &test_ctx()).await;
        assert_eq!(
            collected.tools.iter().map(|t| t.name()).collect::<Vec<_>>(),
            expected.iter().map(|(n, _, _)| *n).collect::<Vec<_>>()
        );
        assert_eq!(collected.tool_definitions.len(), expected.len());
        for (definition, (name, id, label)) in collected.tool_definitions.iter().zip(expected) {
            assert_eq!(definition.name(), name);
            let hints = definition.hints();
            assert_eq!(hints.capability_id.as_deref(), Some(id));
            assert_eq!(hints.capability_name.as_deref(), Some(label));
            let ToolDefinition::Builtin(tool) = definition else {
                panic!("expected builtin")
            };
            let schema = if name == "bash" {
                serde_json::json!({"type":"object"})
            } else {
                serde_json::json!({"type":"object","properties":{},"additionalProperties":false})
            };
            assert_eq!(tool.parameters, schema);
        }
    }
}

#[tokio::test]
async fn prompt_collection_preserves_exact_sections_attribution_and_base_order() {
    let registry = fixture_registry();
    let ids = vec!["prompt_tool_fixture".into(), "second_prompt_fixture".into()];
    let collected = collect_capabilities(&ids, &registry, &test_ctx()).await;
    let first = "<capability id=\"prompt_tool_fixture\">\nTask Management uses the write_todos tool.\n</capability>";
    let second = "<capability id=\"second_prompt_fixture\">\nA second capability prompt contribution.\n</capability>";
    assert_eq!(collected.system_prompt_parts, vec![first, second]);
    assert_eq!(
        collected.system_prompt_attributions,
        vec![
            SystemPromptAttribution {
                capability_id: ids[0].clone(),
                content: first.into()
            },
            SystemPromptAttribution {
                capability_id: ids[1].clone(),
                content: second.into()
            }
        ]
    );
    assert_eq!(
        collected.system_prompt_prefix(),
        Some(format!("{first}\n\n{second}"))
    );
    let applied = apply_capabilities(
        RuntimeAgent::new("Base.", "fixture-model"),
        &ids,
        &registry,
        &test_ctx(),
    )
    .await;
    assert_eq!(
        applied.runtime_agent.system_prompt,
        format!("<system-prompt>\nBase.\n</system-prompt>\n\n{first}\n\n{second}")
    );
    assert!(applied.tool_registry.has("write_todos"));
    assert_eq!(applied.tool_registry.len(), 1);
    for (base, addition, expected) in [
        ("Base.", None, "Base."),
        ("Base.", Some(""), "Base."),
        ("", Some("Extra."), "Extra."),
        (
            "<system-prompt>Base.</system-prompt>",
            Some("Extra."),
            "<system-prompt>Base.</system-prompt>\n\nExtra.",
        ),
    ] {
        assert_eq!(compose_system_prompt(base, addition), expected);
    }
}

#[test]
fn feature_projection_preserves_order_and_distinct_dependency_features() {
    let mut registry = CapabilityRegistry::new();
    registry.register(DependencyFixture {
        id: "base".into(),
        deps: vec![],
        features: vec!["base-only", "shared"],
    });
    registry.register(DependencyFixture {
        id: "parent".into(),
        deps: vec!["base"],
        features: vec!["parent-only", "shared"],
    });
    registry.register(DependencyFixture {
        id: "other".into(),
        deps: vec![],
        features: vec!["other-only"],
    });
    assert_eq!(
        compute_features(&["parent".into()], &registry),
        vec!["base-only", "shared", "parent-only"]
    );
    assert_eq!(
        compute_features(
            &[
                "other".into(),
                "parent".into(),
                "base".into(),
                "parent".into()
            ],
            &registry
        ),
        vec!["other-only", "base-only", "shared", "parent-only"]
    );
}

#[test]
fn dependency_limit_accepts_one_hundred_and_rejects_one_hundred_one() {
    let mut registry = CapabilityRegistry::new();
    let ids: Vec<_> = (0..101).map(|i| format!("cap-{i}")).collect();
    for id in &ids {
        registry.register(DependencyFixture {
            id: id.clone(),
            deps: vec![],
            features: vec![],
        });
    }
    let resolved = resolve_dependencies(&ids[..100], &registry).unwrap();
    assert_eq!(resolved.resolved_ids, ids[..100]);
    assert_eq!(resolved.user_selected, ids[..100]);
    assert!(resolved.added_as_dependencies.is_empty());
    assert_eq!(
        resolve_dependencies(&ids, &registry).unwrap_err(),
        DependencyError::TooManyCapabilities {
            count: 101,
            max: 100
        }
    );
}
