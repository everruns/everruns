//! An agent that states where its commands run before it runs any.
//!
//! The example materializes a small bundled project into a throwaway working
//! copy, mounts it as the agent's `/workspace`, and hands the agent one of two
//! environments: a sandboxed Bashkit shell, or file tools with no execution at
//! all. Before the turn it prints the resulting environment profile — target,
//! containment, durability, and the capability set — which is the same view
//! `GET /v1/sessions/{id}/environment` returns, resolved in-process.
//!
//! The point is the profile, not the task. The same audit succeeds in both
//! environments; what changes is what the agent is able to do to get there,
//! and the profile says so up front rather than leaving it to be discovered
//! from a failing tool call.
//!
//! ```text
//! OPENAI_API_KEY=... cargo run -p everruns-environment-agent
//! OPENAI_API_KEY=... cargo run -p everruns-environment-agent -- --environment files
//! OPENAI_API_KEY=... cargo run -p everruns-environment-agent -- --targets
//! ```

// Terminal presentation is shared by every example; this file is the agent.
use everruns_example_demo::shell as demo;

mod agent;
mod environment;
mod sample_project;

use std::fs;
use std::path::{Path, PathBuf};

use everruns::{ComputeCapabilities, Durability, Engine, NetworkPolicy, OpenAI};

use crate::agent::Runnable;
use crate::environment::{ALL_TARGETS, Profile, Target};

const REPORT: &str = "AUDIT.md";

fn audit_request() -> String {
    format!(
        "Audit the project in /workspace.\n\
         1. Count the TODO markers in every file under src/.\n\
         2. Write /workspace/{REPORT} with one '- <path>: <count>' line for each file that has \
            at least one, ordered by count descending, and omit files with none.\n\
         3. End the file with a 'Total: <n>' line giving the sum.\n\
         4. Report the counts you found."
    )
}

/// Print the environment exactly as a caller would read it from the API.
fn show_profile(profile: &Profile) {
    demo::section("ENVIRONMENT");
    match (&profile.kind, profile.provider) {
        (Some(kind), Some(provider)) => demo::field("target", &format!("{kind} ({provider})")),
        (Some(kind), None) => demo::field("target", &kind.to_string()),
        // Absent, not "none": a files-only session has no target, and saying so
        // is the honest answer rather than a placeholder.
        (None, _) => demo::field("target", "— nothing executes"),
    }
    demo::field(
        "containment",
        &format!(
            "{} · network {}",
            profile.containment.level,
            network(&profile.containment.network)
        ),
    );
    demo::field("durability", durability(profile.durability));
    demo::field("resolved from", profile.resolved_from);
    if let Some(source) = profile.source_capability {
        demo::field("source capability", source);
    }

    demo::section("CAPABILITIES");
    for (label, supported) in capability_rows(&profile.capabilities) {
        demo::check(supported, label);
    }
}

/// The API spells these in snake_case; matching it keeps the example and the
/// HTTP surface readable as one thing.
fn durability(durability: Durability) -> &'static str {
    match durability {
        Durability::Checkpointed => "checkpointed",
        Durability::ProviderSnapshot => "provider_snapshot",
        Durability::None => "none",
    }
}

fn network(policy: &NetworkPolicy) -> String {
    match policy {
        NetworkPolicy::Deny => "deny".to_string(),
        NetworkPolicy::Allow => "allow".to_string(),
        NetworkPolicy::Allowlist(hosts) => format!("allowlist ({})", hosts.join(", ")),
    }
}

fn capability_rows(capabilities: &ComputeCapabilities) -> [(&'static str, bool); 6] {
    [
        ("native processes", capabilities.native_processes),
        ("package installs", capabilities.packages),
        ("pty", capabilities.pty),
        ("listening ports", capabilities.ports),
        ("portable checkpoint", capabilities.portable_checkpoint),
        ("network policy enforced", capabilities.network_enforced),
    ]
}

/// Every target this build can offer, and a reason for each it cannot.
fn show_targets() {
    demo::banner("everruns · environment targets");
    for target in ALL_TARGETS {
        let profile = environment::resolve(target);
        demo::section(target.as_str());
        match environment::unavailable_reason(target) {
            None => demo::check(true, "available"),
            Some(reason) => demo::check(false, reason),
        }
        demo::field("containment", &profile.containment.level.to_string());
        demo::field("durability", durability(profile.durability));
        demo::field(
            "native processes",
            if profile.capabilities.native_processes {
                "yes"
            } else {
                "no"
            },
        );
    }
}

/// One post-run assertion about the report on disk.
struct Check {
    passed: bool,
    label: String,
}

fn check(passed: bool, label: impl Into<String>) -> Check {
    Check {
        passed,
        label: label.into(),
    }
}

/// Re-read the working copy and confirm the audit the agent reported.
///
/// The final answer is not treated as proof. The expected total comes from the
/// embedded fixture, so it cannot drift from the files the agent was given.
fn verify(root: &Path) -> Vec<Check> {
    let report = fs::read_to_string(root.join(REPORT)).unwrap_or_default();
    let mut checks = vec![check(!report.is_empty(), format!("{REPORT} exists"))];

    checks.push(check(
        report.contains("src/client.rs"),
        format!("{REPORT} lists src/client.rs"),
    ));
    checks.push(check(
        report.contains("src/retry.rs"),
        format!("{REPORT} lists src/retry.rs"),
    ));
    checks.push(check(
        !report.contains("src/cache.rs"),
        format!("{REPORT} omits src/cache.rs, which has no markers"),
    ));
    let total = sample_project::todo_total();
    checks.push(check(
        report.contains(&format!("Total: {total}")),
        format!("{REPORT} totals {total} markers"),
    ));
    // Ordering is part of the task, so check it rather than trusting the prose.
    checks.push(check(
        match (report.find("src/client.rs"), report.find("src/retry.rs")) {
            (Some(client), Some(retry)) => client < retry,
            _ => false,
        },
        "the busiest file is listed first",
    ));
    checks
}

fn parse_target(arguments: &[String]) -> Result<Target, String> {
    let mut selected = Target::Bashkit;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--environment" | "-e" => {
                let value = arguments
                    .get(index + 1)
                    .ok_or("--environment needs a value: bashkit, files, or host")?;
                selected =
                    Target::parse(value).ok_or_else(|| format!("unknown environment: {value}"))?;
                index += 2;
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(selected)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--targets") {
        show_targets();
        return Ok(());
    }

    let target = parse_target(&arguments)?;
    let profile = environment::resolve(target);

    demo::banner("everruns · environment agent");
    demo::field("model", agent::MODEL);
    show_profile(&profile);

    // An unavailable target refuses with its reason rather than quietly
    // falling back to one that works.
    let Some(runnable) = Runnable::from_target(target) else {
        let reason =
            environment::unavailable_reason(target).unwrap_or("this target cannot run an agent");
        return Err(format!(
            "cannot run in the {} environment: {reason}",
            target.as_str()
        )
        .into());
    };

    let workspace = tempfile::tempdir()?;
    let root: PathBuf = workspace.path().join("fetchkit");
    fs::create_dir_all(&root)?;
    sample_project::materialize(&root)?;
    demo::field("workspace", &root.display().to_string());

    // `OpenAI::from_env` reads OPENAI_API_KEY; the turn below is a real
    // provider call, not a simulation.
    let agent = agent::build(OpenAI::from_env()?, &root, runnable)?;
    let session = Engine::new().create(agent);
    demo::run(&session, &audit_request()).await?;

    let checks = verify(&root);
    demo::section("AUDIT CHECKS ON DISK");
    for item in &checks {
        demo::check(item.passed, &item.label);
    }
    demo::section(&format!("{REPORT} AFTER THE RUN"));
    demo::body(
        &fs::read_to_string(root.join(REPORT)).unwrap_or_else(|_| "(not written)".into()),
        demo::DIM,
    );

    let failed = checks.iter().filter(|item| !item.passed).count();
    if failed > 0 {
        return Err(format!("{failed} audit check(s) failed").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_environment_is_the_sandbox() {
        assert_eq!(parse_target(&[]).unwrap(), Target::Bashkit);
    }

    #[test]
    fn an_environment_can_be_selected_by_name() {
        let arguments = vec!["--environment".to_string(), "files".to_string()];
        assert_eq!(parse_target(&arguments).unwrap(), Target::Files);
    }

    #[test]
    fn an_unknown_environment_is_refused_rather_than_defaulted() {
        let arguments = vec!["--environment".to_string(), "daytona".to_string()];
        assert!(parse_target(&arguments).is_err());
        assert!(parse_target(&["--environment".to_string()]).is_err());
    }

    #[test]
    fn a_fresh_working_copy_fails_every_audit_check_but_the_absent_file() {
        let workspace = tempfile::tempdir().expect("temp dir");
        sample_project::materialize(workspace.path()).expect("materialize");
        let checks = verify(workspace.path());
        for item in &checks {
            // Only the "omits cache.rs" check can pass before the run, because
            // there is no report to list anything at all.
            let expected = item.label.contains("omits src/cache.rs");
            assert_eq!(
                item.passed, expected,
                "unexpected pre-run state for check: {}",
                item.label
            );
        }
    }

    #[test]
    fn a_correct_report_passes_every_check() {
        let workspace = tempfile::tempdir().expect("temp dir");
        sample_project::materialize(workspace.path()).expect("materialize");
        fs::write(
            workspace.path().join(REPORT),
            "- src/client.rs: 3\n- src/retry.rs: 1\n\nTotal: 4\n",
        )
        .expect("write report");
        assert!(verify(workspace.path()).iter().all(|item| item.passed));
    }

    #[test]
    fn a_report_that_lists_the_files_in_the_wrong_order_fails() {
        let workspace = tempfile::tempdir().expect("temp dir");
        sample_project::materialize(workspace.path()).expect("materialize");
        fs::write(
            workspace.path().join(REPORT),
            "- src/retry.rs: 1\n- src/client.rs: 3\n\nTotal: 4\n",
        )
        .expect("write report");
        let checks = verify(workspace.path());
        assert!(
            checks
                .iter()
                .any(|item| !item.passed && item.label.contains("busiest file"))
        );
    }
}
