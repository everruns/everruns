use super::*;
use crate::domains::sandbox_templates::record::{
    SandboxBootstrap, SandboxLifecycle, SandboxTargetSpec,
};
use std::collections::BTreeMap;

fn profile(
    target: crate::domains::sandbox_templates::record::SandboxTargetSpec,
) -> SandboxTemplateSpec {
    SandboxTemplateSpec {
        template_revision_id: None,
        target,
        containment: None,
        durability: None,
        lifecycle: SandboxLifecycle::default(),
        bootstrap: SandboxBootstrap::default(),
    }
}

#[test]
fn named_profile_resolves_and_replaces_legacy_compute_capabilities() {
    let set = SandboxPolicy {
        mode: None,
        default: "scratch".to_string(),
        templates: BTreeMap::from([(
            "scratch".to_string(),
            profile(SandboxTargetSpec::vfs("bashkit")),
        )]),
    };
    let selected = resolve_sandbox_selection(Some(&set), None)
        .unwrap()
        .unwrap();
    assert_eq!(selected.name, "scratch");
    assert_eq!(selected.spec.durability, SandboxDurability::Checkpointed);

    let capabilities = vec![
        CapabilityRef::new("current_time"),
        CapabilityRef::new("daytona"),
    ];
    let mapped = apply_sandbox_to_capabilities(&capabilities, &[], Some(&selected.spec));
    assert_eq!(
        mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>(),
        vec!["current_time", "bashkit_shell"]
    );
}

#[test]
fn fixed_policy_rejects_even_the_default_session_override() {
    let set = SandboxPolicy {
        mode: Some(SandboxPolicyMode::Fixed),
        default: "scratch".to_string(),
        templates: BTreeMap::from([(
            "scratch".to_string(),
            profile(SandboxTargetSpec::vfs("bashkit")),
        )]),
    };
    let error = resolve_sandbox_selection(
        Some(&set),
        Some(&SandboxSelection::Named {
            r#use: "scratch".into(),
        }),
    )
    .unwrap_err();
    assert!(error.contains("fixed Sandbox Template"));
}

#[test]
fn selectable_policy_rejects_inline_profiles() {
    let selected = profile(SandboxTargetSpec::vfs("bashkit"));
    let set = SandboxPolicy {
        mode: Some(SandboxPolicyMode::Selectable),
        default: "scratch".to_string(),
        templates: BTreeMap::from([("scratch".to_string(), selected.clone())]),
    };
    let error = resolve_sandbox_selection(
        Some(&set),
        Some(&SandboxSelection::Inline(Box::new(selected))),
    )
    .unwrap_err();
    assert!(error.contains("declared Sandbox Template"));
}

#[test]
fn fixed_policy_requires_exactly_one_profile() {
    let selected = profile(SandboxTargetSpec::vfs("bashkit"));
    let set = SandboxPolicy {
        mode: Some(SandboxPolicyMode::Fixed),
        default: "one".to_string(),
        templates: BTreeMap::from([
            ("one".to_string(), selected.clone()),
            ("two".to_string(), selected),
        ]),
    };

    assert!(
        validate_sandbox_policy(&set)
            .unwrap_err()
            .contains("exactly one")
    );
}

#[test]
fn userless_session_uses_the_sole_organization_account() {
    let accounts = vec![("brave_search", 1), ("daytona", 2), ("e2b", 3)];
    assert_eq!(
        sole_organization_account("daytona", accounts, |a| a.0).unwrap(),
        ("daytona", 2)
    );
}

#[test]
fn userless_session_fails_without_exactly_one_organization_account() {
    let none = sole_organization_account("daytona", vec![("e2b", 1)], |a| a.0).unwrap_err();
    assert!(none.contains("no daytona account"), "{none}");

    let many = sole_organization_account("daytona", vec![("daytona", 1), ("daytona", 2)], |a| a.0)
        .unwrap_err();
    assert!(many.contains("has 2 daytona accounts"), "{many}");
}

#[test]
fn managed_profile_replaces_the_session_filesystem_tool_surface() {
    let resolved = resolve_spec(&profile(SandboxTargetSpec::managed("daytona"))).unwrap();
    assert_eq!(
        resolved.target.credential.source,
        SessionSandboxCredentialSource::SessionUser
    );
    let capabilities = vec![
        CapabilityRef::new("session_file_system"),
        CapabilityRef::new("bashkit_shell"),
        CapabilityRef::new("current_time"),
    ];

    let mapped = apply_sandbox_to_capabilities(&capabilities, &[], Some(&resolved));

    assert_eq!(
        mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>(),
        vec!["current_time", "session_sandbox"]
    );
}

/// Sandbox Worker inherits Worker's session file system. Under a managed
/// template the sandbox must own every file tool the model can call, or a
/// file the sandbox just cloned reads as "File not found".
#[tokio::test]
async fn sandbox_worker_under_a_managed_template_exposes_no_vfs_file_tool() {
    use everruns_contracts::capability::BuiltInHarnessPreset;
    use everruns_core::capabilities::{
        SESSION_FILE_TOOL_NAMES, SystemPromptContext, collect_capabilities_with_configs,
        resolve_capability_configs,
    };

    let managed = resolve_spec(&profile(SandboxTargetSpec::managed("daytona"))).unwrap();
    // Sandbox Worker adds no capability of its own; Worker is its whole set.
    let harness = BuiltInHarnessPreset::Worker.effective_capabilities();
    let session = apply_sandbox_to_capabilities(&[], &harness, Some(&managed));
    let effective = everruns_core::merge_capabilities(&harness, &session);
    let registry = crate::platform::oss_capability_registry_for_grade(DeploymentGrade::Dev);
    let resolved = resolve_capability_configs(&effective, &registry).unwrap();
    let ctx =
        SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
    let collected = collect_capabilities_with_configs(&resolved, &registry, &ctx).await;

    // Skills and output persistence still need the VFS capability.
    assert!(
        collected
            .applied_ids
            .iter()
            .any(|id| id == "session_file_system")
    );
    let mut definitions = collected.tool_definitions;
    for hook in &collected.tool_definition_hooks {
        definitions = hook.transform(definitions);
    }
    for name in SESSION_FILE_TOOL_NAMES {
        let matching: Vec<_> = definitions.iter().filter(|d| d.name() == *name).collect();
        assert!(matching.len() <= 1, "{name} offered twice");
        if let Some(definition) = matching.first() {
            assert_eq!(
                definition.hints().capability_id.as_deref(),
                Some("session_sandbox"),
                "{name} must run in the sandbox"
            );
        }
        let tool = collected.tools.iter().rfind(|t| t.name() == *name);
        assert_eq!(
            tool.is_some(),
            !matching.is_empty(),
            "{name}: definition and implementation disagree"
        );
        if let Some(tool) = tool {
            assert_eq!(
                tool.to_definition().concurrency_class(),
                Some("session_sandbox"),
                "{name} must execute in the sandbox"
            );
        }
    }
    for name in [
        "read_file",
        "read_many_files",
        "list_directory",
        "glob",
        "grep",
    ] {
        assert!(
            definitions.iter().any(|d| d.name() == name),
            "{name} missing"
        );
    }
}

#[test]
fn bashkit_profile_keeps_the_agents_http_opt_in() {
    let resolved = resolve_spec(&profile(SandboxTargetSpec::vfs("bashkit"))).unwrap();
    let shell = |enable_http: bool| {
        CapabilityRef::with_config("bashkit_shell", json!({ "enable_http": enable_http }))
    };
    let http_of = |mapped: Vec<CapabilityRef>| {
        mapped
            .iter()
            .find(|capability| capability.id() == "bashkit_shell")
            .map(|capability| capability.config_value()["enable_http"].clone())
    };

    // The agent layer opted in: the pinned Bashkit profile keeps it.
    let agent = vec![CapabilityRef::new("current_time"), shell(true)];
    assert_eq!(
        http_of(apply_sandbox_to_capabilities(&[], &agent, Some(&resolved))),
        Some(json!(true))
    );
    // A session layer that says no wins over the agent.
    assert_eq!(
        http_of(apply_sandbox_to_capabilities(
            &[shell(false)],
            &agent,
            Some(&resolved)
        )),
        Some(json!(false))
    );
    // Nobody opted in: HTTP stays off.
    assert_eq!(
        http_of(apply_sandbox_to_capabilities(
            &[],
            &[CapabilityRef::new("bashkit_shell")],
            Some(&resolved)
        )),
        Some(json!(false))
    );
    // A managed profile never inherits the Bashkit opt-in.
    let managed = resolve_spec(&profile(SandboxTargetSpec::managed("daytona"))).unwrap();
    assert_eq!(
        http_of(apply_sandbox_to_capabilities(&[], &agent, Some(&managed))),
        None
    );
}

#[test]
fn organization_credential_requires_an_exact_connection() {
    let mut value = profile(SandboxTargetSpec::managed("daytona"));
    value.target.credential.source = SessionSandboxCredentialSource::Organization;
    assert!(resolve_spec(&value).unwrap_err().contains("connection_id"));

    value.target.credential.connection_id = Some(uuid::Uuid::new_v4());
    let resolved = resolve_spec(&value).unwrap();
    assert_eq!(
        capability_for_sandbox(&resolved).config_value()["credential"]["source"],
        "organization"
    );
}

#[test]
fn non_managed_target_rejects_provider_credentials() {
    let mut value = profile(SandboxTargetSpec::vfs("bashkit"));
    value.target.credential.source = SessionSandboxCredentialSource::Agent;
    assert!(
        resolve_spec(&value)
            .unwrap_err()
            .contains("does not accept")
    );
}

#[test]
fn profile_rejects_embedded_credentials() {
    let mut value = profile(SandboxTargetSpec::managed("daytona"));
    value.target.options = json!({"api_key": "do-not-store-me"});
    assert!(
        resolve_spec(&value)
            .unwrap_err()
            .contains("connection reference")
    );
}

#[test]
fn profile_rejects_provider_endpoint_overrides() {
    let mut value = profile(SandboxTargetSpec::managed("daytona"));
    value.target.options = json!({"api_base": "https://attacker.invalid"});
    assert!(
        resolve_spec(&value)
            .unwrap_err()
            .contains("not caller-configurable")
    );
}

#[test]
fn checkpointed_daytona_profile_enables_portable_recovery() {
    let mut value = profile(SandboxTargetSpec::managed("daytona"));
    value.durability = Some(SandboxDurability::Checkpointed);
    let resolved = resolve_spec(&value).unwrap();
    let capability = capability_for_sandbox(&resolved);

    assert_eq!(capability.id(), "session_sandbox");
    assert_eq!(
        capability.config_value()["provider_config"]["recovery"]["enabled"],
        true
    );
    assert_eq!(
        capability.config_value()["provider_config"]["workspace_path"],
        "/home/daytona/workspace"
    );
}

#[test]
fn modal_profile_maps_to_a_session_sandbox_with_its_options() {
    let mut value = profile(SandboxTargetSpec::managed("modal"));
    value.target.options = json!({
        "runtime": "gvisor", "image": "node:22", "cpu": 0.5, "memory_mb": 1024,
        "workspace_path": "/srv/app", "title": "web"
    });
    let resolved = resolve_spec(&value).unwrap();
    assert_eq!(resolved.durability, SandboxDurability::ProviderSnapshot);
    let capability = capability_for_sandbox(&resolved);
    let config = capability.config_value();
    assert_eq!(capability.id(), "session_sandbox");
    assert_eq!(config["provider"], "modal");
    assert_eq!(config["provider_config"]["runtime"], "gvisor");
    assert_eq!(config["provider_config"]["workspace_path"], "/srv/app");
    assert!(config["provider_config"].get("recovery").is_none());
}

#[test]
fn boxd_profile_maps_to_a_session_sandbox_with_region_and_size() {
    let mut value = profile(SandboxTargetSpec::managed("boxd"));
    value.target.options = json!({
        "region": "nyc", "size": "4x16", "workspace_path": "/srv/app", "title": "web"
    });
    value.containment = Some(SandboxContainmentSpec {
        network: SandboxNetworkPolicy::Allowlist {
            allowed_hosts: vec!["github.com".into()],
        },
        ..SandboxContainmentSpec::isolated()
    });
    let resolved = resolve_spec(&value).unwrap();
    assert_eq!(resolved.durability, SandboxDurability::ProviderSnapshot);
    let capability = capability_for_sandbox(&resolved);
    let config = capability.config_value();
    assert_eq!(capability.id(), "session_sandbox");
    assert_eq!(config["provider"], "boxd");
    assert_eq!(config["provider_config"]["region"], "nyc");
    assert_eq!(config["provider_config"]["size"], "4x16");
    assert_eq!(
        config["provider_config"]["network"],
        json!({"mode": "allowlist", "domains": ["github.com"]})
    );
    assert!(config["provider_config"].get("recovery").is_none());
}

#[test]
fn isolated_sandbox_profile_runs_on_the_platform_credential() {
    let mut value = profile(SandboxTargetSpec::managed("kubernetes"));
    value.target.options = json!({"size": "medium", "desktop": true, "title": "web"});
    value.containment = Some(SandboxContainmentSpec {
        network: SandboxNetworkPolicy::Allowlist {
            allowed_hosts: vec!["10.0.0.0/8".into()],
        },
        ..SandboxContainmentSpec::isolated()
    });
    let resolved = resolve_spec(&value).unwrap();
    assert_eq!(resolved.durability, SandboxDurability::ProviderSnapshot);
    assert_eq!(
        resolved.target.credential.source,
        SessionSandboxCredentialSource::Platform
    );
    let capability = capability_for_sandbox(&resolved);
    let config = capability.config_value();
    assert_eq!(config["provider"], "kubernetes");
    assert_eq!(config["provider_config"]["size"], "medium");
}

#[test]
fn isolated_sandbox_profile_rejects_what_the_cluster_cannot_enforce() {
    for (options, network, needle) in [
        (json!({"size": "huge"}), None, "size"),
        (json!({"image": "ubuntu; rm -rf /"}), None, "image"),
        (json!({"region": "nyc"}), None, "not caller-configurable"),
        (json!({"workspace_path": "/a/../b"}), None, "workspace_path"),
        (
            json!({}),
            Some(SandboxNetworkPolicy::Allowlist {
                allowed_hosts: vec!["github.com".into()],
            }),
            "domain allowlist",
        ),
        (
            json!({}),
            Some(SandboxNetworkPolicy::Allowlist {
                allowed_hosts: vec!["https://10.0.0.1/admin".into()],
            }),
            "IP ranges",
        ),
        (
            json!({}),
            Some(SandboxNetworkPolicy::Allowlist {
                allowed_hosts: vec!["10.0.0.0/33".into()],
            }),
            "IP ranges",
        ),
    ] {
        let mut value = profile(SandboxTargetSpec::managed("kubernetes"));
        value.target.options = options.clone();
        if let Some(network) = network {
            value.containment = Some(SandboxContainmentSpec {
                network,
                ..SandboxContainmentSpec::isolated()
            });
        }
        let error = resolve_spec(&value).unwrap_err();
        assert!(error.contains(needle), "{options}: {error}");
    }

    let mut checkpointed = profile(SandboxTargetSpec::managed("kubernetes"));
    checkpointed.durability = Some(SandboxDurability::Checkpointed);
    assert!(
        resolve_spec(&checkpointed)
            .unwrap_err()
            .contains("provider_snapshot")
    );

    // A user's or organization's account never runs this target, and
    // other providers never run on the platform credential.
    let mut user = profile(SandboxTargetSpec::managed("kubernetes"));
    user.target.credential.source = SessionSandboxCredentialSource::SessionUser;
    assert!(resolve_spec(&user).unwrap_err().contains("platform"));
    let mut daytona = profile(SandboxTargetSpec::managed("daytona"));
    daytona.target.credential.source = SessionSandboxCredentialSource::Platform;
    assert!(resolve_spec(&daytona).unwrap_err().contains("platform"));
}

#[test]
fn boxd_profile_rejects_what_boxd_would_refuse() {
    for (options, needle) in [
        (json!({"region": "lon"}), "region"),
        (json!({"size": "large"}), "size"),
        (
            json!({"snapshot": "base", "size": "2x8"}),
            "cannot be combined",
        ),
        (json!({"desktop": true}), "not caller-configurable"),
        (json!({"image": "node:22"}), "not caller-configurable"),
        (json!({"workspace_path": "relative"}), "workspace_path"),
    ] {
        let mut value = profile(SandboxTargetSpec::managed("boxd"));
        value.target.options = options.clone();
        let error = resolve_spec(&value).unwrap_err();
        assert!(error.contains(needle), "{options}: {error}");
    }

    let mut checkpointed = profile(SandboxTargetSpec::managed("boxd"));
    checkpointed.durability = Some(SandboxDurability::Checkpointed);
    assert!(
        resolve_spec(&checkpointed)
            .unwrap_err()
            .contains("provider_snapshot")
    );

    // boxd machines have no IPv6 egress.
    let mut value = profile(SandboxTargetSpec::managed("boxd"));
    value.containment = Some(SandboxContainmentSpec {
        network: SandboxNetworkPolicy::Allowlist {
            allowed_hosts: vec!["2001:db8::1".to_string()],
        },
        ..SandboxContainmentSpec::isolated()
    });
    let error = resolve_spec(&value).unwrap_err();
    assert!(error.contains("IPv6"), "{error}");
}

#[test]
fn modal_profile_rejects_what_the_provider_cannot_do() {
    let cases = [
        (json!({"snapshot": "x"}), "not caller-configurable"),
        (
            json!({"_test_server_url": "http://x"}),
            "not caller-configurable",
        ),
        (json!({"runtime": "firecracker"}), "vm or gvisor"),
        (json!({"cpu": 0}), "cpu"),
        (json!({"memory_mb": 64}), "memory_mb"),
        (json!({"workspace_path": "/a/../b"}), "workspace_path"),
        (json!({"image": ""}), "image"),
    ];
    for (options, expected) in cases {
        let mut value = profile(SandboxTargetSpec::managed("modal"));
        value.target.options = options.clone();
        let error = resolve_spec(&value).unwrap_err();
        assert!(error.contains(expected), "{options}: {error}");
    }

    let mut checkpointed = profile(SandboxTargetSpec::managed("modal"));
    checkpointed.durability = Some(SandboxDurability::Checkpointed);
    assert!(
        resolve_spec(&checkpointed)
            .unwrap_err()
            .contains("provider_snapshot")
    );

    let unknown = profile(SandboxTargetSpec::managed("fly"));
    assert!(
        resolve_spec(&unknown)
            .unwrap_err()
            .contains("boxd, daytona, e2b, kubernetes, modal")
    );
}

#[test]
fn e2b_profile_maps_provider_options_and_rejects_checkpointed_durability() {
    let mut value = profile(SandboxTargetSpec::managed("e2b"));
    value.target.options = json!({
        "template": "base",
        "timeout_seconds": 3600,
        "workspace_path": "/home/user/workspace"
    });
    let resolved = resolve_spec(&value).unwrap();
    let config = capability_for_sandbox(&resolved).config_value().clone();
    assert_eq!(config["provider"], "e2b");
    assert_eq!(config["provider_config"]["template"], "base");
    assert_eq!(config["provider_config"]["timeout_seconds"], 3600);

    value.durability = Some(SandboxDurability::Checkpointed);
    assert!(
        resolve_spec(&value)
            .unwrap_err()
            .contains("provider_snapshot")
    );
}

#[test]
fn managed_providers_enforce_the_template_network_policy() {
    let modal = |network: SandboxNetworkPolicy, options: Value| {
        let mut value = profile(SandboxTargetSpec::managed("modal"));
        value.containment = Some(SandboxContainmentSpec {
            network,
            ..SandboxContainmentSpec::isolated()
        });
        value.target.options = options;
        resolve_spec(&value)
    };
    let config = |resolved: &ResolvedSandboxSpec| {
        capability_for_sandbox(resolved).config_value()["provider_config"].clone()
    };

    let denied = modal(SandboxNetworkPolicy::Deny, json!({})).unwrap();
    assert_eq!(config(&denied)["network"], json!({"mode": "blocked"}));
    let listed = modal(
        SandboxNetworkPolicy::Allowlist {
            allowed_hosts: vec!["pypi.org".into(), "*.pythonhosted.org".into()],
        },
        json!({}),
    )
    .unwrap();
    assert_eq!(
        config(&listed)["network"],
        json!({"mode": "allowlist", "domains": ["pypi.org", "*.pythonhosted.org"]})
    );
    let injected = modal(
        SandboxNetworkPolicy::Allow,
        json!({"inject_connections": ["github"]}),
    )
    .unwrap();
    assert_eq!(config(&injected)["network"], json!({"mode": "open"}));
    assert_eq!(config(&injected)["inject_connections"], json!(["github"]));

    for (network, options, expected) in [
        (
            SandboxNetworkPolicy::Deny,
            json!({"inject_connections": ["github"]}),
            "needs network",
        ),
        (
            SandboxNetworkPolicy::Allowlist {
                allowed_hosts: vec!["github.com".into()],
            },
            json!({"inject_connections": ["github"]}),
            "limited to domains",
        ),
        (
            SandboxNetworkPolicy::Allow,
            json!({"inject_connections": ["slack"]}),
            "cannot be injected",
        ),
        (
            SandboxNetworkPolicy::Allowlist {
                allowed_hosts: vec!["localhost".into()],
            },
            json!({}),
            "not a domain",
        ),
    ] {
        let error = modal(network, options).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }

    // Daytona and E2B enforce deny and allowlists too.
    let restricted = |provider: &str, network: SandboxNetworkPolicy| {
        let mut value = profile(SandboxTargetSpec::managed(provider));
        value.containment = Some(SandboxContainmentSpec {
            network,
            ..SandboxContainmentSpec::isolated()
        });
        resolve_spec(&value)
    };
    let daytona = restricted(
        "daytona",
        SandboxNetworkPolicy::Allowlist {
            allowed_hosts: vec!["github.com".into()],
        },
    )
    .unwrap();
    assert_eq!(
        config(&daytona)["network"],
        json!({"mode": "allowlist", "domains": ["github.com"]})
    );
    let daytona = restricted("daytona", SandboxNetworkPolicy::Deny).unwrap();
    assert_eq!(config(&daytona)["network"], json!({"mode": "blocked"}));
    let e2b = restricted("e2b", SandboxNetworkPolicy::Deny).unwrap();
    assert_eq!(config(&e2b)["network"], json!({"mode": "blocked"}));
}

#[test]
fn modal_is_offered_only_at_development_grade() {
    assert!(managed_provider_offered("modal", DeploymentGrade::Dev));
    assert!(!managed_provider_offered("modal", DeploymentGrade::Prod));
    assert!(managed_provider_offered("daytona", DeploymentGrade::Prod));

    let resolved = resolve_spec(&profile(SandboxTargetSpec::managed("modal"))).unwrap();
    assert!(validate_target_available_for_grade(&resolved, DeploymentGrade::Dev).is_ok());
    assert!(
        validate_target_available_for_grade(&resolved, DeploymentGrade::Prod)
            .unwrap_err()
            .contains("development grade")
    );
}

#[test]
fn default_must_name_a_profile() {
    let set = SandboxPolicy {
        mode: None,
        default: "missing".to_string(),
        templates: BTreeMap::from([(
            "scratch".to_string(),
            profile(SandboxTargetSpec::vfs("bashkit")),
        )]),
    };
    assert!(
        validate_sandbox_policy(&set)
            .unwrap_err()
            .contains("does not name")
    );
}

#[test]
fn desktop_option_adds_the_sandbox_desktop_and_replaces_other_displays() {
    for provider in ["daytona", "e2b"] {
        let mut value = profile(SandboxTargetSpec::managed(provider));
        value.target.options = json!({"desktop": {"display_width": 1024, "display_height": 768}});
        let resolved = resolve_spec(&value).unwrap();
        let mapped = apply_sandbox_to_capabilities(
            &[
                CapabilityRef::new("computer_use"),
                CapabilityRef::new("web_fetch"),
            ],
            &[],
            Some(&resolved),
        );
        let ids = mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>();
        assert_eq!(ids, ["web_fetch", "session_sandbox", "sandbox_desktop"]);
        // The desktop drives the same Sandbox: one config for both.
        assert_eq!(mapped[1].config_value(), mapped[2].config_value());
        assert_eq!(
            mapped[2].config_value()["provider_config"]["desktop"]["display_width"],
            1024
        );
    }
}

#[test]
fn templates_without_a_desktop_keep_their_computer_use_capability() {
    for desktop in [None, Some(json!(false))] {
        let mut value = profile(SandboxTargetSpec::managed("e2b"));
        if let Some(desktop) = desktop {
            value.target.options = json!({ "desktop": desktop });
        }
        let resolved = resolve_spec(&value).unwrap();
        let mapped = apply_sandbox_to_capabilities(
            &[CapabilityRef::new("computer_use")],
            &[],
            Some(&resolved),
        );
        let ids = mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>();
        assert_eq!(ids, ["computer_use", "session_sandbox"]);
    }
}

#[test]
fn desktop_accepts_baked_snapshot_without_losing_desktop_settings() {
    let mut value = profile(SandboxTargetSpec::managed("daytona"));
    value.target.options = json!({"desktop":true,"snapshot":"baked-desktop"});
    let resolved = resolve_spec(&value).unwrap();
    assert_eq!(resolved.target.options["snapshot"], "baked-desktop");
    assert_eq!(resolved.target.options["desktop"], true);
}

#[test]
fn desktop_option_rejects_bad_settings_and_unsupported_providers() {
    let rejected = |provider: &str, options: Value| {
        let mut value = profile(SandboxTargetSpec::managed(provider));
        value.target.options = options;
        resolve_spec(&value).unwrap_err()
    };
    assert!(rejected("e2b", json!({"desktop": "yes"})).contains("desktop must be"));
    assert!(
        rejected("e2b", json!({"desktop": {"display_width": 99_999}}))
            .contains("desktop is not valid")
    );
    assert!(
        rejected("daytona", json!({"desktop": true, "size": "small"}))
            .contains("cannot be combined with desktop")
    );
    assert!(rejected("modal", json!({"desktop": true})).contains("desktop"));
    assert!(
        rejected("daytona", json!({"desktop": {"record": "no"}}))
            .contains("desktop.record must be true or false")
    );
}

#[test]
fn desktop_option_accepts_turning_the_recording_off() {
    let mut value = profile(SandboxTargetSpec::managed("daytona"));
    value.target.options = json!({"desktop": {"record": false, "display_width": 1280}});
    assert!(resolve_spec(&value).is_ok());
}
