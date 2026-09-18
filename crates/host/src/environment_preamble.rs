//! The environment preamble, derived from the environment (EVE-1042).
//!
//! # Why derived
//!
//! Harnesses used to hand-write prose describing the world they run in, and
//! that prose drifted from the world. `coding-container`, `coding-daytona` and
//! `coding-session-sandbox` differed in one capability each and then repeated
//! roughly a hundred near-identical lines, with provider tool names spelled
//! into the text — `sandbox_exec` against `daytona_exec` against
//! `sandbox_read_file`.
//!
//! A written prompt can disagree with the environment. A derived one cannot.
//! [`Environment`](crate::Environment) already carries every fact the preamble
//! states: what the target is, what it can do, what contains it, and what
//! survives losing it. This turns those facts into the sentences a model needs
//! and nothing else.
//!
//! # What it deliberately does not say
//!
//! **No tool names.** Which tool performs an action is the tool list's job, and
//! it is the part that rotted fastest — a prompt naming `daytona_exec` is wrong
//! the moment the same harness is bound to a container. The preamble describes
//! the world; the tool schemas describe the verbs.
//!
//! **No behavior.** How to approach a coding task, when to commit, how to
//! phrase an answer: none of that follows from the environment, so none of it
//! belongs here. That stays in the harness prompt, which is what `system_prompt`
//! is left for.
//!
//! See `knowledge/harnesses/execution-environments.md`.

use crate::compute::{
    ComputeCapabilities, ComputeKind, Containment, ContainmentLevel, Durability, NetworkPolicy,
};

/// How many allowlisted hosts to name before summarising the rest.
///
/// A long allowlist is configuration, not orientation: past a handful the model
/// does not need the list, it needs to know one exists.
const MAX_NAMED_HOSTS: usize = 5;

/// The facts a preamble is rendered from.
///
/// Taken as a struct rather than read off [`Environment`](crate::Environment)
/// directly so the rendering can be tested against every combination without
/// standing up a compute provider, and so a caller that knows its facts by
/// other means can render the same sentences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentFacts {
    /// `None` for a file-only environment, where nothing runs.
    pub kind: Option<ComputeKind>,
    pub capabilities: ComputeCapabilities,
    pub containment: Containment,
    pub durability: Durability,
}

/// Render the environment preamble.
///
/// Returns `None` when there is nothing to say — a caller then contributes no
/// preamble at all rather than a paragraph of "you cannot do anything", which
/// spends tokens telling a model about verbs it was never offered.
pub fn environment_preamble(facts: &EnvironmentFacts) -> Option<String> {
    let kind = facts.kind?;

    let mut lines = vec![format!("## Environment\n\n{}", where_commands_run(kind))];

    if let Some(sentence) = what_it_can_do(&facts.capabilities) {
        lines.push(sentence);
    }
    lines.push(containment_sentence(&facts.containment));
    lines.push(network_sentence(&facts.containment.network));
    lines.push(durability_sentence(facts.durability));

    Some(lines.join("\n\n"))
}

fn where_commands_run(kind: ComputeKind) -> &'static str {
    match kind {
        ComputeKind::Host => {
            "Commands run directly on the machine hosting this session. It is a real machine you \
             share with whatever else runs on it."
        }
        ComputeKind::Machine => {
            "Commands run on a registered remote machine reached over the network. It is a real \
             machine that outlives this session."
        }
        ComputeKind::Vfs => {
            "Commands run in an interpreter over this session's own filesystem. There is no \
             operating system underneath: only what the interpreter implements exists."
        }
        ComputeKind::Container => {
            "Commands run in a container created for this session, with its own filesystem and \
             process table."
        }
        ComputeKind::Managed => {
            "Commands run in a provider-managed remote sandbox created for this session, with its \
             own filesystem and process table."
        }
    }
}

/// What the target can do, stated as capabilities rather than as tools.
///
/// Absences are stated as plainly as presences. A target that cannot run native
/// binaries is the load-bearing case: a model that assumes it can reads every
/// failure as a broken environment rather than a limited one, and keeps
/// retrying.
fn what_it_can_do(capabilities: &ComputeCapabilities) -> Option<String> {
    let mut can: Vec<&str> = Vec::new();
    let mut cannot: Vec<&str> = Vec::new();

    for (supported, yes, no) in [
        (
            capabilities.native_processes,
            "run native binaries and child processes",
            "run native binaries — only what the interpreter implements is available",
        ),
        (
            capabilities.packages,
            "install packages that persist across commands",
            "install packages",
        ),
        (
            capabilities.pty,
            "attach an interactive terminal",
            "attach an interactive terminal, so commands must not expect one",
        ),
        (
            capabilities.ports,
            "listen on ports and reach them",
            "listen on ports that anything can reach",
        ),
    ] {
        if supported {
            can.push(yes);
        } else {
            cannot.push(no);
        }
    }

    let mut parts = Vec::new();
    if !can.is_empty() {
        parts.push(format!("You can {}.", join_clauses(&can)));
    }
    if !cannot.is_empty() {
        parts.push(format!("You cannot {}.", join_clauses(&cannot)));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn containment_sentence(containment: &Containment) -> String {
    let mut sentence = match containment.level {
        ContainmentLevel::None => {
            "Nothing isolates these commands from the host, so treat destructive operations as \
             destructive to a real machine."
                .to_string()
        }
        ContainmentLevel::Native => {
            "A kernel policy bounds what these commands may touch.".to_string()
        }
        ContainmentLevel::Isolated => {
            "A separate kernel, VM, or interpreter bounds these commands; the host is not \
             reachable from here."
                .to_string()
        }
    };

    if !containment.writable_roots.is_empty() {
        sentence.push_str(&format!(
            " Writes are confined to {}.",
            join_clauses(
                &containment
                    .writable_roots
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            )
        ));
    }
    sentence
}

fn network_sentence(network: &NetworkPolicy) -> String {
    match network {
        NetworkPolicy::Deny => {
            "Outbound network access is denied. Anything needed from the network must already be \
             present."
                .to_string()
        }
        NetworkPolicy::Allow => "Outbound network access is open.".to_string(),
        NetworkPolicy::Allowlist(hosts) if hosts.is_empty() => {
            "Outbound network access is restricted to an allowlist, and nothing is on it."
                .to_string()
        }
        NetworkPolicy::Allowlist(hosts) if hosts.len() <= MAX_NAMED_HOSTS => format!(
            "Outbound network access is restricted to {}.",
            join_clauses(&hosts.iter().map(String::as_str).collect::<Vec<_>>())
        ),
        NetworkPolicy::Allowlist(hosts) => format!(
            "Outbound network access is restricted to an allowlist of {} hosts; a request to \
             anything else will fail.",
            hosts.len()
        ),
    }
}

fn durability_sentence(durability: Durability) -> String {
    match durability {
        Durability::Checkpointed => {
            "This filesystem is checkpointed, so work here survives losing the compute.".to_string()
        }
        Durability::ProviderSnapshot => {
            "The provider can snapshot this filesystem, which makes a restart fast but is not a \
             guarantee. Anything that must survive belongs in session storage."
                .to_string()
        }
        Durability::None => {
            "Nothing recovers this filesystem if the compute is lost. Anything that must survive \
             belongs in session storage."
                .to_string()
        }
    }
}

/// `a`, `a and b`, `a, b, and c`.
fn join_clauses(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(kind: ComputeKind) -> EnvironmentFacts {
        EnvironmentFacts {
            kind: Some(kind),
            capabilities: ComputeCapabilities::full_machine(),
            containment: Containment::isolated().network(NetworkPolicy::Allow),
            durability: Durability::None,
        }
    }

    /// A file-only environment contributes nothing rather than a paragraph
    /// about verbs the model was never offered.
    #[test]
    fn an_environment_with_no_compute_says_nothing() {
        let facts = EnvironmentFacts {
            kind: None,
            ..facts(ComputeKind::Vfs)
        };
        assert!(environment_preamble(&facts).is_none());
    }

    /// The whole point: the same harness on two targets describes each
    /// correctly, because the sentences come from the target.
    #[test]
    fn the_same_facts_except_the_target_read_differently() {
        let container = environment_preamble(&facts(ComputeKind::Container)).expect("preamble");
        let managed = environment_preamble(&facts(ComputeKind::Managed)).expect("preamble");
        let vfs = environment_preamble(&facts(ComputeKind::Vfs)).expect("preamble");

        assert!(container.contains("container"), "{container}");
        assert!(managed.contains("provider-managed"), "{managed}");
        assert!(vfs.contains("interpreter"), "{vfs}");
        assert_ne!(container, managed);
        assert_ne!(container, vfs);
    }

    /// The load-bearing absence. A model that assumes it can run binaries reads
    /// every failure as a broken environment and keeps retrying.
    #[test]
    fn a_target_without_native_processes_says_so() {
        let facts = EnvironmentFacts {
            capabilities: ComputeCapabilities::default(),
            ..facts(ComputeKind::Vfs)
        };
        let preamble = environment_preamble(&facts).expect("preamble");
        assert!(preamble.contains("cannot"), "{preamble}");
        assert!(preamble.contains("native binaries"), "{preamble}");
    }

    #[test]
    fn a_full_machine_states_what_it_can_do_and_claims_no_limits() {
        let preamble = environment_preamble(&facts(ComputeKind::Container)).expect("preamble");
        assert!(
            preamble.contains("You can run native binaries"),
            "{preamble}"
        );
        assert!(
            !preamble.contains("You cannot"),
            "a full machine has nothing to withhold: {preamble}"
        );
    }

    #[test]
    fn containment_levels_read_differently() {
        let mut open = facts(ComputeKind::Host);
        open.containment = Containment::none();
        let open = environment_preamble(&open).expect("preamble");
        assert!(open.contains("Nothing isolates"), "{open}");
        assert!(
            open.contains("destructive to a real machine"),
            "an uncontained host must say so: {open}"
        );

        let isolated = environment_preamble(&facts(ComputeKind::Container)).expect("preamble");
        assert!(isolated.contains("host is not reachable"), "{isolated}");
    }

    #[test]
    fn the_network_policy_is_stated() {
        let mut denied = facts(ComputeKind::Container);
        denied.containment = Containment::isolated().network(NetworkPolicy::Deny);
        assert!(
            environment_preamble(&denied)
                .expect("preamble")
                .contains("denied"),
        );

        let mut allowlisted = facts(ComputeKind::Container);
        allowlisted.containment = Containment::isolated().network(NetworkPolicy::Allowlist(vec![
            "crates.io".to_string(),
            "github.com".to_string(),
        ]));
        let text = environment_preamble(&allowlisted).expect("preamble");
        assert!(text.contains("crates.io and github.com"), "{text}");
    }

    /// A long allowlist is configuration, not orientation.
    #[test]
    fn a_long_allowlist_is_summarised_rather_than_recited() {
        let hosts: Vec<String> = (0..40).map(|i| format!("host-{i}.example")).collect();
        let mut facts = facts(ComputeKind::Container);
        facts.containment = Containment::isolated().network(NetworkPolicy::Allowlist(hosts));
        let text = environment_preamble(&facts).expect("preamble");
        assert!(text.contains("40 hosts"), "{text}");
        assert!(!text.contains("host-7.example"), "{text}");
    }

    #[test]
    fn durability_tells_the_model_what_survives() {
        for (durability, expected) in [
            (Durability::Checkpointed, "survives losing the compute"),
            (Durability::ProviderSnapshot, "not a guarantee"),
            (Durability::None, "Nothing recovers"),
        ] {
            let mut facts = facts(ComputeKind::Container);
            facts.durability = durability;
            let text = environment_preamble(&facts).expect("preamble");
            assert!(text.contains(expected), "{durability:?}: {text}");
        }
    }

    /// The regression this issue exists to prevent. A provider tool name in the
    /// preamble is wrong the moment the harness is bound to another target.
    #[test]
    fn no_provider_tool_name_reaches_the_preamble() {
        let forbidden = [
            "sandbox_exec",
            "sandbox_read_file",
            "sandbox_write_file",
            "daytona_exec",
            "daytona_read_file",
            "daytona_create_sandbox",
            "read_file",
            "write_file",
        ];
        for kind in [
            ComputeKind::Host,
            ComputeKind::Machine,
            ComputeKind::Vfs,
            ComputeKind::Container,
            ComputeKind::Managed,
        ] {
            let text = environment_preamble(&facts(kind)).expect("preamble");
            for name in forbidden {
                assert!(
                    !text.contains(name),
                    "{kind:?} preamble names the tool {name}: {text}"
                );
            }
        }
    }

    #[test]
    fn clauses_join_readably() {
        assert_eq!(join_clauses(&[]), "");
        assert_eq!(join_clauses(&["a"]), "a");
        assert_eq!(join_clauses(&["a", "b"]), "a and b");
        assert_eq!(join_clauses(&["a", "b", "c"]), "a, b, and c");
    }
}
