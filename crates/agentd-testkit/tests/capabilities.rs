//! Which of the hosted capabilities a shard runs (data plane v2, step
//! 11.6b).
//!
//! Every capability in the hosted catalog (`docs/api/capability-catalog.json`,
//! generated from the server's registry) is named, with its default config,
//! by an agent on the shard's host. The agent either builds and completes a
//! turn on a fleet, or is refused with the reason the shard gives. The
//! outcome per capability is kept in `capability_matrix.json` next to this
//! file, and the test fails when the shard's behavior changes from it.
//!
//! Decisions:
//! - The matrix is checked, not hand-kept: a capability the shard starts or
//!   stops serving shows up here as a diff. Regenerate it with
//!   `UPDATE_CAPABILITY_MATRIX=1 cargo test -p agentd-testkit --test
//!   capabilities`; the variable only rewrites the file, the test always runs.
//! - A turn runs on a simulated model that answers without calling a tool,
//!   so "runs" means the capability builds and activates in a shard turn.
//!   Calling each tool is the live check's (11.6c).
//! - The fleet has no control plane, so "runs" also means it runs with
//!   none (parity, Phase A): what a capability's tools still ask a control
//!   plane at call time is listed beside it under `host_calls`, from its
//!   declaration (`Capability::host_calls`, with the capabilities it
//!   depends on), and every name there must be a declared host call.
//!   `sandbox_fleet` is the one that cannot finish a turn with none: its
//!   sandboxes are the session's resources, which only a control plane
//!   keeps, so it fails here as on a Framework host, and `lookups.rs`
//!   covers it against one (parity, step 10).
//! - A capability a deployment turns on by flag (`FEATURE_<NAME>`: Docker,
//!   the container sandbox, paid Parallel) is turned on here, as an
//!   operator would for both planes; one with required settings gets the
//!   smallest valid config ([`config_for`]).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use everruns_agentd::Host;
use serde_json::{Value, json};
use support::Fleet;

const CATALOG: &str = include_str!("../../../docs/api/capability-catalog.json");

fn matrix_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/capability_matrix.json")
}

/// The ids the hosted catalog lists.
fn catalog() -> Vec<String> {
    let catalog: Value = serde_json::from_str(CATALOG).unwrap();
    catalog["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Turn on every capability a deployment can enable by flag, as an
/// operator would for both planes (`FEATURE_<NAME>`), so the matrix records
/// what a shard does with each, not what this deployment's grade hides.
fn deployment_flags() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: set once, before this binary starts any fleet thread or
        // builds the hosted catalog.
        unsafe {
            for flag in ["DOCKER_CAPABILITY", "CONTAINER_SANDBOX", "MACHINE_PAYMENTS"] {
                std::env::set_var(format!("FEATURE_{flag}"), "internal");
            }
        }
    });
}

/// The smallest valid config of a capability that has required settings.
fn config_for(id: &str) -> Option<Value> {
    Some(match id {
        "dreaming" => json!({
            "destinations": [{ "mount": "/memory/user", "path": "/notes.md" }]
        }),
        "sandbox_desktop" => json!({
            "provider": "daytona",
            "provider_config": { "desktop": true }
        }),
        _ => return None,
    })
}

fn agent_with(id: &str) -> Value {
    let mut definition = everruns_agentd_testkit::definition("echo").unwrap();
    definition["name"] = json!(format!("with-{}", id.replace('_', "-")));
    definition["capabilities"] = json!([match config_for(id) {
        Some(config) => json!({ "ref": id, "config": config }),
        None => json!({ "ref": id }),
    }]);
    definition
}

/// The first line of a refusal, without the parts that vary per run.
fn reason(error: &str) -> String {
    error.lines().next().unwrap_or(error).trim().to_owned()
}

/// Run one turn of `definition`'s agent; `None` when it completed, or why
/// it did not.
async fn one_turn(fleet: &Fleet, definition: &Value) -> Option<String> {
    let (status, _, created) = fleet
        .post_once("/v1/sessions", json!({ "agent": definition }))
        .await;
    if status != 201 && status != 200 {
        return Some(format!("session refused ({status}): {}", created["error"]));
    }
    let session = created["id"].as_str().unwrap().to_owned();
    fleet.say(&session, "hello").await;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let events = fleet.events(&session).await;
        if let Some(failed) = events.iter().find(|e| e["type"] == "turn.failed") {
            let error = failed["data"]["error"].as_str().unwrap_or("turn failed");
            return Some(format!("turn failed: {}", reason(error)));
        }
        if events.iter().any(|e| e["type"] == "turn.completed") {
            return None;
        }
        if Instant::now() > deadline {
            return Some("turn did not finish in 30 s".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_shard_runs_the_capabilities_its_matrix_records() {
    deployment_flags();
    let host = everruns_agentd_testkit::host();
    let mut fleet = Fleet::start().await;
    fleet.executor(1);

    let mut found = BTreeMap::new();
    let mut host_calls = BTreeMap::new();
    for id in catalog() {
        let definition = agent_with(&id);
        let declared = everruns_agentd::host::declared_host_calls(&definition);
        for name in &declared {
            assert!(
                everruns_contracts::host_calls::by_name(name).is_some(),
                "{id} declares {name}, which is not a host call"
            );
        }
        if !declared.is_empty() {
            host_calls.insert(id.clone(), declared);
        }
        let outcome = match host.agent(&definition) {
            Err(error) => format!("refused: {}", reason(&error)),
            Ok(_) => match one_turn(&fleet, &definition).await {
                None => "runs".to_owned(),
                Some(why) => why,
            },
        };
        found.insert(id, outcome);
    }
    let found = json!({
        "_comment": "Generated by crates/shard-testkit/tests/capabilities.rs. Do not edit by hand.",
        "capabilities": found,
        "host_calls": host_calls,
    });

    let path = matrix_path();
    if std::env::var_os("UPDATE_CAPABILITY_MATRIX").is_some() {
        let mut text = serde_json::to_string_pretty(&found).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_owned()))
            .unwrap();
    assert_eq!(
        recorded["capabilities"],
        found["capabilities"],
        "the shard's capabilities changed; regenerate {} with UPDATE_CAPABILITY_MATRIX=1",
        path.display()
    );
    assert_eq!(
        recorded["host_calls"],
        found["host_calls"],
        "the capabilities' host calls changed; regenerate {} with UPDATE_CAPABILITY_MATRIX=1",
        path.display()
    );
}
