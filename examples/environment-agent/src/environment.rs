//! Which environment a session runs in, derived from the capabilities it carries.
//!
//! Environment profiles are not stored configuration yet, so this is the same
//! derivation the control plane performs for `GET /v1/sessions/{id}/environment`,
//! written against the Framework's own types. Keeping the rules in one readable
//! place is the point: a caller should be able to learn that Bashkit cannot run
//! native binaries *before* the first turn, rather than from a confusing tool
//! error in the middle of one.

use everruns::{ComputeCapabilities, ComputeKind, Containment, Durability, NetworkPolicy};

/// The environments this example can select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The sandboxed Bashkit shell over the session filesystem.
    Bashkit,
    /// File tools only. Nothing executes, which is a real configuration.
    Files,
    /// Commands on the machine this process already runs on.
    Host,
}

impl Target {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "bashkit" => Some(Self::Bashkit),
            "files" => Some(Self::Files),
            "host" => Some(Self::Host),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bashkit => "bashkit",
            Self::Files => "files",
            Self::Host => "host",
        }
    }
}

/// What a caller learns about a session's environment before it runs.
///
/// The field names match the API response deliberately; this is the same view,
/// resolved in-process instead of over HTTP.
#[derive(Clone, Debug)]
pub struct Profile {
    /// Absent when nothing executes. `None` is the honest answer for a
    /// files-only session, not a gap to fill with a plausible default.
    pub kind: Option<ComputeKind>,
    pub provider: Option<&'static str>,
    pub containment: Containment,
    pub durability: Durability,
    pub capabilities: ComputeCapabilities,
    /// Why the environment is what it is. Always `capabilities` today, because
    /// profiles are derived rather than stored. The field says so rather than
    /// implying configuration that does not exist.
    pub resolved_from: &'static str,
    pub source_capability: Option<&'static str>,
}

/// Bashkit interprets a bash subset against the session filesystem. It runs no
/// native binaries at all, which is the single most important thing a caller
/// can know about it, and its filesystem is the durable Everruns VFS.
pub fn bashkit_capabilities() -> ComputeCapabilities {
    ComputeCapabilities {
        native_processes: false,
        packages: false,
        pty: false,
        ports: false,
        portable_checkpoint: true,
        network_enforced: true,
    }
}

/// Resolve the profile for a target.
pub fn resolve(target: Target) -> Profile {
    match target {
        Target::Bashkit => Profile {
            kind: Some(ComputeKind::Vfs),
            provider: Some("bashkit"),
            // Bashkit is an interpreter boundary, and Everruns egress is
            // default-deny. An allowlist is a profile field that does not
            // exist yet, so reporting one would be fiction.
            containment: Containment::isolated().network(NetworkPolicy::Deny),
            durability: Durability::Checkpointed,
            capabilities: bashkit_capabilities(),
            resolved_from: "capabilities",
            source_capability: Some("bashkit_shell"),
        },
        // Saying "isolated" here would invent a boundary around something that
        // never runs, so containment is `none` and every capability is false.
        Target::Files => Profile {
            kind: None,
            provider: None,
            containment: Containment::none().network(NetworkPolicy::Deny),
            durability: Durability::Checkpointed,
            capabilities: ComputeCapabilities::default(),
            resolved_from: "capabilities",
            source_capability: None,
        },
        Target::Host => host_profile(),
    }
}

/// The host profile, read from `HostCompute` itself when it is compiled in.
///
/// Built with `--features host-compute` this is not a table of claims: the
/// values come off the `Compute` trait, so the example cannot drift from what
/// the target actually reports.
#[cfg(feature = "host-compute")]
fn host_profile() -> Profile {
    use everruns::{Compute, HostCompute};

    // The root only scopes where commands would run; the capability,
    // containment, and durability answers below are properties of the target
    // itself, so describing it from the current directory is enough.
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let compute = HostCompute::new(root);
    Profile {
        kind: Some(compute.kind()),
        provider: None,
        containment: match compute.enforced_containment() {
            everruns::ContainmentLevel::Isolated => Containment::isolated(),
            everruns::ContainmentLevel::Native => Containment::native(),
            everruns::ContainmentLevel::None => Containment::none(),
        },
        durability: compute.durability(),
        capabilities: compute.capabilities(),
        resolved_from: "compute",
        source_capability: None,
    }
}

#[cfg(not(feature = "host-compute"))]
fn host_profile() -> Profile {
    Profile {
        kind: Some(ComputeKind::Host),
        provider: None,
        containment: Containment::none(),
        durability: Durability::None,
        capabilities: ComputeCapabilities::full_machine(),
        resolved_from: "unavailable",
        source_capability: None,
    }
}

/// Why a target cannot be selected, or `None` when it can.
///
/// Availability is asked, never assumed, and an unavailable target owes a
/// reason rather than quietly disappearing from the list.
pub fn unavailable_reason(target: Target) -> Option<&'static str> {
    match target {
        Target::Bashkit | Target::Files => None,
        #[cfg(feature = "host-compute")]
        Target::Host => Some(
            "HostCompute is built, but no capability routes agent tool calls through \
             Environment::compute() yet, so the agent would have no shell",
        ),
        #[cfg(not(feature = "host-compute"))]
        Target::Host => Some("rebuild with --features host-compute to construct the host target"),
    }
}

pub const ALL_TARGETS: [Target; 3] = [Target::Bashkit, Target::Files, Target::Host];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bashkit_never_claims_native_processes() {
        let profile = resolve(Target::Bashkit);
        assert!(!profile.capabilities.native_processes);
        // The load-bearing pair: no native binaries, but a portable checkpoint.
        assert!(profile.capabilities.portable_checkpoint);
        assert_eq!(profile.durability, Durability::Checkpointed);
        assert_eq!(profile.kind, Some(ComputeKind::Vfs));
    }

    #[test]
    fn a_files_only_session_reports_no_target_rather_than_a_contained_one() {
        let profile = resolve(Target::Files);
        assert_eq!(profile.kind, None);
        assert_eq!(profile.capabilities, ComputeCapabilities::default());
        assert_eq!(
            profile.containment,
            Containment::none().network(NetworkPolicy::Deny)
        );
    }

    #[test]
    fn the_host_target_contains_nothing_and_recovers_nothing() {
        let profile = resolve(Target::Host);
        assert_eq!(profile.containment.level, everruns::ContainmentLevel::None);
        assert!(profile.capabilities.native_processes);
        // Everruns does not own the hardware, so loss is loss.
        #[cfg(feature = "host-compute")]
        assert_eq!(profile.durability, Durability::None);
    }

    #[test]
    fn every_selectable_target_is_either_available_or_explains_itself() {
        for target in ALL_TARGETS {
            let runnable = matches!(target, Target::Bashkit | Target::Files);
            assert_eq!(unavailable_reason(target).is_none(), runnable);
        }
    }

    #[test]
    fn target_names_round_trip() {
        for target in ALL_TARGETS {
            assert_eq!(Target::parse(target.as_str()), Some(target));
        }
        assert_eq!(Target::parse("daytona"), None);
    }
}
