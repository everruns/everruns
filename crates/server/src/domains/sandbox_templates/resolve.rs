// Capabilities -> primary Sandbox view.
//
// New Sessions pin a resolved Sandbox specification. Older Sessions have no
// snapshot, so their compatibility view is derived from the capability that
// supplies compute. The mapping lives here so the stored and legacy views do
// not drift into different opinions about what Bashkit can do.
//
// Every entry is deliberately pessimistic: a capability nobody has taught this
// table about contributes no compute rather than a plausible-looking guess.

use crate::records::{SandboxContainmentLevel, SandboxDurability, SandboxNetworkPolicy};
use everruns_contracts::capability::CapabilityRef;
use everruns_contracts::typed_id::SandboxId;

use crate::api::sandbox_templates::{
    SandboxCapabilities, SandboxContainment, SandboxTarget, SandboxTargetDescriptor,
    SessionSandboxResponse,
};

/// Capability ids that supply compute, in precedence order.
///
/// Precedence matters: a harness carrying both a managed sandbox and the
/// in-process shell runs its real work in the sandbox, so that is the
/// Sandbox to report.
const COMPUTE_CAPABILITIES: &[&str] = &[
    "session_sandbox",
    "container_sandbox",
    "daytona",
    "e2b",
    "docker_container",
    "bashkit_shell",
];

fn full_machine() -> SandboxCapabilities {
    SandboxCapabilities {
        native_processes: true,
        packages: true,
        pty: true,
        ports: true,
        portable_checkpoint: false,
        network_enforced: false,
    }
}

/// Bashkit interprets a bash subset against the session filesystem. It runs no
/// native binaries at all, which is the single most important thing a caller
/// can know about it, and its filesystem is the durable Everruns VFS.
fn bashkit_capabilities() -> SandboxCapabilities {
    SandboxCapabilities {
        native_processes: false,
        packages: false,
        pty: false,
        ports: false,
        portable_checkpoint: true,
        network_enforced: true,
    }
}

fn managed_capabilities(portable_checkpoint: bool) -> SandboxCapabilities {
    SandboxCapabilities {
        portable_checkpoint,
        ..full_machine()
    }
}

/// The provider a `session_sandbox` capability is configured with.
fn session_sandbox_provider(config: &serde_json::Value) -> Option<String> {
    config
        .get("provider")
        .and_then(|provider| provider.as_str())
        .map(str::to_string)
}

/// Whether a `session_sandbox` capability was configured with an
/// Everruns-owned recovery volume. Without one there is no portable
/// checkpoint, only whatever the provider snapshots for itself.
fn session_sandbox_recovery_enabled(config: &serde_json::Value) -> bool {
    config
        .get("provider_config")
        .and_then(|value| value.get("recovery"))
        .and_then(|recovery| recovery.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn bashkit_http_enabled(config: &serde_json::Value) -> bool {
    config
        .get("enable_http")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Derive the primary Sandbox a Session runs in from its effective capabilities.
pub fn sandbox_from_capabilities(capabilities: &[CapabilityRef]) -> SessionSandboxResponse {
    let compute = COMPUTE_CAPABILITIES.iter().find_map(|wanted| {
        capabilities
            .iter()
            .find(|capability| capability.id() == *wanted)
    });

    let Some(capability) = compute else {
        // No compute is a real configuration: an agent that only reads and
        // writes files needs none, and saying "isolated" here would invent a
        // boundary around something that never runs.
        return SessionSandboxResponse {
            sandbox_id: None,
            sandbox_template_revision_id: None,
            role: None,
            name: None,
            target: None,
            containment: SandboxContainment {
                level: "none".to_string(),
                network: "deny".to_string(),
            },
            durability: "checkpointed".to_string(),
            capabilities: SandboxCapabilities::default(),
            resolved_from: "capabilities".to_string(),
            source_capability: None,
            spec: None,
            desired_state: None,
            observed_state: None,
            generation: None,
            current_checkpoint_id: None,
            last_activity_at: None,
        };
    };

    let config = capability.config_value();
    let (kind, provider, caps, durability): (&str, Option<String>, SandboxCapabilities, &str) =
        match capability.id() {
            "bashkit_shell" => (
                "vfs",
                Some("bashkit".to_string()),
                bashkit_capabilities(),
                "checkpointed",
            ),
            "session_sandbox" => {
                let recovery = session_sandbox_recovery_enabled(config);
                (
                    "managed",
                    session_sandbox_provider(config),
                    managed_capabilities(recovery),
                    if recovery {
                        "checkpointed"
                    } else {
                        "provider_snapshot"
                    },
                )
            }
            "daytona" => (
                "managed",
                Some("daytona".to_string()),
                managed_capabilities(false),
                "provider_snapshot",
            ),
            "e2b" => (
                "managed",
                Some("e2b".to_string()),
                managed_capabilities(false),
                "provider_snapshot",
            ),
            "container_sandbox" | "docker_container" => (
                "container",
                Some("docker".to_string()),
                managed_capabilities(false),
                "provider_snapshot",
            ),
            // Unreachable while this list and COMPUTE_CAPABILITIES agree, and
            // harmless if they ever stop agreeing.
            _ => ("managed", None, SandboxCapabilities::default(), "none"),
        };

    let network = match capability.id() {
        "bashkit_shell" if !bashkit_http_enabled(config) => "deny",
        _ => "allow",
    };

    SessionSandboxResponse {
        target: Some(SandboxTarget {
            kind: kind.to_string(),
            provider,
            connection_id: None,
        }),
        containment: SandboxContainment {
            level: "isolated".to_string(),
            network: network.to_string(),
        },
        durability: durability.to_string(),
        capabilities: caps,
        resolved_from: "capabilities".to_string(),
        source_capability: Some(capability.id().to_string()),
        sandbox_id: None,
        sandbox_template_revision_id: None,
        role: None,
        name: None,
        spec: None,
        desired_state: None,
        observed_state: None,
        generation: None,
        current_checkpoint_id: None,
        last_activity_at: None,
    }
}

/// Render a stored logical Sandbox. Capability-derived feature flags stay
/// grounded in the actual adapter, while policy and lifecycle come from the
/// immutable specification rather than being guessed from tool names.
pub fn sandbox_from_record(
    record: &crate::storage::PrimarySandboxRecord,
    capabilities: &[CapabilityRef],
) -> SessionSandboxResponse {
    let mut response = sandbox_from_capabilities(capabilities);
    let spec = &record.spec;
    let sandbox_id = SandboxId::from_uuid(record.id).to_string();
    response.sandbox_id = Some(sandbox_id);
    response.sandbox_template_revision_id =
        record.sandbox_template_revision_id.map(|id| id.to_string());
    response.role = Some("primary".to_string());
    response.name = Some(record.binding_name.clone());
    response.target = Some(SandboxTarget {
        kind: spec.target.kind.as_str().to_string(),
        provider: spec.target.provider.clone(),
        connection_id: spec.target.connection_id.clone(),
    });
    response.containment = SandboxContainment {
        level: match spec.containment.level {
            SandboxContainmentLevel::None => "none",
            SandboxContainmentLevel::Native => "native",
            SandboxContainmentLevel::Isolated => "isolated",
        }
        .to_string(),
        network: match spec.containment.network {
            SandboxNetworkPolicy::Deny => "deny",
            SandboxNetworkPolicy::Allowlist { .. } => "allowlist",
            SandboxNetworkPolicy::Allow => "allow",
        }
        .to_string(),
    };
    response.durability = match spec.durability {
        SandboxDurability::Checkpointed => "checkpointed",
        SandboxDurability::ProviderSnapshot => "provider_snapshot",
        SandboxDurability::None => "none",
    }
    .to_string();
    response.resolved_from = "spec".to_string();
    response.spec = Some(spec.clone());
    response.desired_state = Some(record.desired_state.clone());
    response.observed_state = Some(record.observed_state.clone());
    response.generation = Some(record.generation);
    response.current_checkpoint_id = record.current_checkpoint_id.map(|id| id.to_string());
    response.last_activity_at = record.last_activity_at;
    response
}

/// The targets this deployment can offer.
///
/// Availability is asked, never assumed: a managed provider counts only when
/// its plugin is actually registered in this binary.
pub fn sandbox_targets() -> Vec<SandboxTargetDescriptor> {
    sandbox_targets_for_grade(everruns_core::DeploymentGrade::from_env())
}

fn sandbox_targets_for_grade(
    grade: everruns_core::DeploymentGrade,
) -> Vec<SandboxTargetDescriptor> {
    let registered = |provider: &str| {
        everruns_capabilities::session_sandbox::create_session_sandbox_provider(provider).is_some()
    };
    let daytona_registered = registered("daytona");
    let e2b_registered = registered("e2b");
    let modal_registered = registered("modal");
    let modal_offered = super::resolution::managed_provider_offered("modal", grade);

    vec![
        SandboxTargetDescriptor {
            display_name: "Bashkit".to_string(),
            icon: "terminal".to_string(),
            kind: "vfs".to_string(),
            provider: Some("bashkit".to_string()),
            available: true,
            reason: None,
            capabilities: bashkit_capabilities(),
            containment_levels: vec!["isolated".to_string()],
            durability: "checkpointed".to_string(),
            credential_sources: vec!["none".to_string()],
        },
        SandboxTargetDescriptor {
            display_name: "Daytona".to_string(),
            icon: "cloud".to_string(),
            kind: "managed".to_string(),
            provider: Some("daytona".to_string()),
            available: daytona_registered,
            reason: (!daytona_registered)
                .then(|| "the Daytona provider is not registered in this deployment".to_string()),
            capabilities: managed_capabilities(true),
            containment_levels: vec!["isolated".to_string()],
            durability: "checkpointed".to_string(),
            credential_sources: vec![
                "session_user".to_string(),
                "agent".to_string(),
                "organization".to_string(),
            ],
        },
        SandboxTargetDescriptor {
            display_name: "E2B".to_string(),
            icon: "cloud".to_string(),
            kind: "managed".to_string(),
            provider: Some("e2b".to_string()),
            available: e2b_registered,
            reason: (!e2b_registered)
                .then(|| "the E2B provider is not registered in this deployment".to_string()),
            capabilities: managed_capabilities(false),
            containment_levels: vec!["isolated".to_string()],
            durability: "provider_snapshot".to_string(),
            credential_sources: vec![
                "session_user".to_string(),
                "agent".to_string(),
                "organization".to_string(),
            ],
        },
        // Modal pauses by snapshotting the filesystem into a provider image;
        // nothing portable leaves Modal.
        SandboxTargetDescriptor {
            display_name: "Modal".to_string(),
            icon: "cloud".to_string(),
            kind: "managed".to_string(),
            provider: Some("modal".to_string()),
            available: modal_registered && modal_offered,
            reason: if !modal_registered {
                Some("the Modal provider is not registered in this deployment".to_string())
            } else if !modal_offered {
                Some("Modal is experimental and offered only at development grade".to_string())
            } else {
                None
            },
            // Modal enforces deny and allowlists outside the sandbox.
            capabilities: SandboxCapabilities {
                network_enforced: true,
                ..managed_capabilities(false)
            },
            containment_levels: vec!["isolated".to_string()],
            durability: "provider_snapshot".to_string(),
            credential_sources: vec![
                "session_user".to_string(),
                "agent".to_string(),
                "organization".to_string(),
            ],
        },
        SandboxTargetDescriptor {
            display_name: "Host".to_string(),
            icon: "server".to_string(),
            kind: "host".to_string(),
            provider: None,
            available: false,
            reason: Some(
                "host execution exists in the Framework but is not wired into the control plane"
                    .to_string(),
            ),
            capabilities: full_machine(),
            containment_levels: vec!["none".to_string()],
            durability: "none".to_string(),
            credential_sources: vec!["none".to_string()],
        },
        SandboxTargetDescriptor {
            display_name: "Registered machine".to_string(),
            icon: "monitor".to_string(),
            kind: "machine".to_string(),
            provider: None,
            available: false,
            reason: Some("registered machines are not implemented yet".to_string()),
            capabilities: full_machine(),
            containment_levels: vec!["none".to_string()],
            durability: "none".to_string(),
            credential_sources: vec!["connection".to_string()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn capability(id: &str) -> CapabilityRef {
        CapabilityRef::new(id)
    }

    #[test]
    fn a_session_with_no_compute_reports_no_target() {
        let view = sandbox_from_capabilities(&[capability("session_file_system")]);

        assert!(view.target.is_none());
        assert!(!view.capabilities.native_processes);
        assert_eq!(view.source_capability, None);
    }

    #[test]
    fn bashkit_never_claims_native_processes() {
        let view = sandbox_from_capabilities(&[
            capability("session_file_system"),
            capability("bashkit_shell"),
        ]);

        let target = view.target.expect("bashkit supplies compute");
        assert_eq!(target.kind, "vfs");
        assert_eq!(target.provider.as_deref(), Some("bashkit"));
        // The load-bearing claim: a caller learns `cargo build` is impossible
        // here before the first turn rather than from a confusing tool error.
        assert!(!view.capabilities.native_processes);
        assert!(view.capabilities.portable_checkpoint);
        assert_eq!(view.durability, "checkpointed");
        assert_eq!(view.containment.network, "deny");
    }

    #[test]
    fn bashkit_reports_network_access_when_http_is_enabled() {
        let view = sandbox_from_capabilities(&[CapabilityRef::with_config(
            "bashkit_shell",
            json!({ "enable_http": true }),
        )]);

        assert_eq!(view.containment.network, "allow");
    }

    #[test]
    fn networked_sandboxes_do_not_claim_to_deny_egress() {
        for capability in [
            CapabilityRef::with_config("session_sandbox", json!({ "provider": "daytona" })),
            capability("daytona"),
            capability("e2b"),
            capability("container_sandbox"),
            capability("docker_container"),
        ] {
            let view = sandbox_from_capabilities(&[capability]);

            assert_eq!(view.containment.network, "allow");
            assert!(!view.capabilities.network_enforced);
        }
    }

    #[test]
    fn a_managed_sandbox_outranks_the_in_process_shell() {
        let view = sandbox_from_capabilities(&[
            capability("bashkit_shell"),
            CapabilityRef::with_config("session_sandbox", json!({ "provider": "daytona" })),
        ]);

        let target = view.target.expect("the sandbox supplies compute");
        assert_eq!(target.kind, "managed");
        assert_eq!(target.provider.as_deref(), Some("daytona"));
        assert!(view.capabilities.native_processes);
        assert_eq!(view.source_capability.as_deref(), Some("session_sandbox"));
    }

    #[test]
    fn recovery_is_what_separates_a_checkpoint_from_a_snapshot() {
        let without = sandbox_from_capabilities(&[CapabilityRef::with_config(
            "session_sandbox",
            json!({ "provider": "daytona" }),
        )]);
        assert_eq!(without.durability, "provider_snapshot");
        assert!(!without.capabilities.portable_checkpoint);

        let with = sandbox_from_capabilities(&[CapabilityRef::with_config(
            "session_sandbox",
            json!({
                "provider": "daytona",
                "provider_config": { "recovery": { "enabled": true } }
            }),
        )]);
        assert_eq!(with.durability, "checkpointed");
        assert!(with.capabilities.portable_checkpoint);
    }

    #[test]
    fn modal_is_listed_but_offered_only_at_development_grade() {
        let modal = |grade| {
            sandbox_targets_for_grade(grade)
                .into_iter()
                .find(|target| target.provider.as_deref() == Some("modal"))
                .expect("modal is listed")
        };
        let dev = modal(everruns_core::DeploymentGrade::Dev);
        assert!(dev.available, "{:?}", dev.reason);
        assert_eq!(dev.durability, "provider_snapshot");
        assert!(!dev.capabilities.portable_checkpoint);
        assert!(dev.capabilities.network_enforced);

        let prod = modal(everruns_core::DeploymentGrade::Prod);
        assert!(!prod.available);
        assert!(prod.reason.unwrap().contains("development grade"));
    }

    #[test]
    fn e2b_is_a_managed_session_sandbox_target() {
        let e2b = sandbox_targets()
            .into_iter()
            .find(|target| target.provider.as_deref() == Some("e2b"))
            .expect("E2B is listed");
        assert!(e2b.available, "{:?}", e2b.reason);
        assert_eq!(e2b.durability, "provider_snapshot");
        assert!(e2b.credential_sources.contains(&"organization".to_string()));
    }

    #[test]
    fn an_unavailable_target_says_why() {
        let targets = sandbox_targets();
        let host = targets
            .iter()
            .find(|target| target.kind == "host")
            .expect("host is listed even when unavailable");

        assert!(!host.available);
        assert!(host.reason.is_some(), "an absent target owes a reason");
        // Nothing may promise recovery from hardware Everruns does not own.
        assert_eq!(host.durability, "none");
    }
}
