// Capabilities -> environment view.
//
// Environments are not stored configuration yet, so the view is derived from
// the capability that supplies a session's compute. The mapping lives here, in
// one place, so the API, the UI, and later the profile validator cannot drift
// into three different opinions about what Bashkit can do.
//
// Every entry is deliberately pessimistic: a capability nobody has taught this
// table about contributes no compute rather than a plausible-looking guess.

use everruns_capability::CapabilityRef;

use crate::api::environments::{
    EnvironmentCapabilities, EnvironmentContainment, EnvironmentTarget,
    EnvironmentTargetDescriptor, SessionEnvironmentResponse,
};

/// Capability ids that supply compute, in precedence order.
///
/// Precedence matters: a harness carrying both a managed sandbox and the
/// in-process shell runs its real work in the sandbox, so that is the
/// environment to report.
const COMPUTE_CAPABILITIES: &[&str] = &[
    "session_sandbox",
    "container_sandbox",
    "daytona",
    "e2b",
    "docker_container",
    "bashkit_shell",
];

fn full_machine() -> EnvironmentCapabilities {
    EnvironmentCapabilities {
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
fn bashkit_capabilities() -> EnvironmentCapabilities {
    EnvironmentCapabilities {
        native_processes: false,
        packages: false,
        pty: false,
        ports: false,
        portable_checkpoint: true,
        network_enforced: true,
    }
}

fn managed_capabilities(portable_checkpoint: bool) -> EnvironmentCapabilities {
    EnvironmentCapabilities {
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

/// Derive the environment a session runs in from its effective capabilities.
pub fn environment_from_capabilities(capabilities: &[CapabilityRef]) -> SessionEnvironmentResponse {
    let compute = COMPUTE_CAPABILITIES.iter().find_map(|wanted| {
        capabilities
            .iter()
            .find(|capability| capability.id() == *wanted)
    });

    let Some(capability) = compute else {
        // No compute is a real configuration: an agent that only reads and
        // writes files needs none, and saying "isolated" here would invent a
        // boundary around something that never runs.
        return SessionEnvironmentResponse {
            target: None,
            containment: EnvironmentContainment {
                level: "none".to_string(),
                network: "deny".to_string(),
            },
            durability: "checkpointed".to_string(),
            capabilities: EnvironmentCapabilities::default(),
            resolved_from: "capabilities".to_string(),
            source_capability: None,
        };
    };

    let config = capability.config_value();
    let (kind, provider, caps, durability): (&str, Option<String>, EnvironmentCapabilities, &str) =
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
            _ => ("managed", None, EnvironmentCapabilities::default(), "none"),
        };

    SessionEnvironmentResponse {
        target: Some(EnvironmentTarget {
            kind: kind.to_string(),
            provider,
        }),
        containment: EnvironmentContainment {
            level: "isolated".to_string(),
            // Every shipping target enforces its own boundary and Everruns
            // egress is default-deny; an allowlist is a profile field that does
            // not exist yet, so reporting one would be fiction.
            network: "deny".to_string(),
        },
        durability: durability.to_string(),
        capabilities: caps,
        resolved_from: "capabilities".to_string(),
        source_capability: Some(capability.id().to_string()),
    }
}

/// The targets this deployment can offer.
///
/// Availability is asked, never assumed: a managed provider counts only when
/// its plugin is actually registered in this binary.
pub fn environment_targets() -> Vec<EnvironmentTargetDescriptor> {
    let daytona_registered =
        everruns_platform::session_sandbox::create_session_sandbox_provider("daytona").is_some();

    vec![
        EnvironmentTargetDescriptor {
            kind: "vfs".to_string(),
            provider: Some("bashkit".to_string()),
            available: true,
            reason: None,
            capabilities: bashkit_capabilities(),
            containment_levels: vec!["isolated".to_string()],
            durability: "checkpointed".to_string(),
        },
        EnvironmentTargetDescriptor {
            kind: "managed".to_string(),
            provider: Some("daytona".to_string()),
            available: daytona_registered,
            reason: (!daytona_registered)
                .then(|| "the Daytona provider is not registered in this deployment".to_string()),
            capabilities: managed_capabilities(true),
            containment_levels: vec!["isolated".to_string()],
            durability: "checkpointed".to_string(),
        },
        EnvironmentTargetDescriptor {
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
        },
        EnvironmentTargetDescriptor {
            kind: "machine".to_string(),
            provider: None,
            available: false,
            reason: Some("registered machines are not implemented yet".to_string()),
            capabilities: full_machine(),
            containment_levels: vec!["none".to_string()],
            durability: "none".to_string(),
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
        let view = environment_from_capabilities(&[capability("session_file_system")]);

        assert!(view.target.is_none());
        assert!(!view.capabilities.native_processes);
        assert_eq!(view.source_capability, None);
    }

    #[test]
    fn bashkit_never_claims_native_processes() {
        let view = environment_from_capabilities(&[
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
    }

    #[test]
    fn a_managed_sandbox_outranks_the_in_process_shell() {
        let view = environment_from_capabilities(&[
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
        let without = environment_from_capabilities(&[CapabilityRef::with_config(
            "session_sandbox",
            json!({ "provider": "daytona" }),
        )]);
        assert_eq!(without.durability, "provider_snapshot");
        assert!(!without.capabilities.portable_checkpoint);

        let with = environment_from_capabilities(&[CapabilityRef::with_config(
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
    fn an_unavailable_target_says_why() {
        let targets = environment_targets();
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
