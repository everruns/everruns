//! The feature mix of the step 11.7 load run (`cell-drive load --agent
//! mix`): half chat, the rest writing and reading session files, setting a
//! schedule, asking a subagent, calling an MCP server (the test node's stub)
//! and using a managed sandbox (the local provider). Every kind of session
//! must finish its turns on a fleet before the run spends EKS time on it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::atomic::Ordering;

use everruns_cell_testkit::stub_mcp;
use serde_json::Value;
use support::{Fleet, answers};

/// The driver's mix, session `i` running entry `i mod 20` (`drive.rs`).
const MIX: &[&str] = &[
    "chat", "files", "chat", "schedule", "chat", "team", "chat", "mcp", "chat", "files", "chat",
    "schedule", "chat", "team", "chat", "mcp", "chat", "files", "chat", "sandbox",
];

#[tokio::test(flavor = "multi_thread")]
async fn every_session_of_the_load_feature_mix_finishes_its_turns() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    fleet.executor(2);
    let mcp_calls = stub_mcp::TOOL_CALLS.load(Ordering::Relaxed);
    let sessions_file = std::env::temp_dir().join(format!("mix-{}.txt", std::process::id()));

    // Two rounds of the twenty-session mix, two turns each.
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_cell-drive"))
        .args([
            "load",
            "--gateways",
            &fleet.bases().join(","),
            "--agent",
            "mix",
            "--sessions",
            "40",
            "--turns",
            "2",
            "--concurrency",
            "40",
            "--sessions-file",
            sessions_file.to_str().unwrap(),
        ])
        .output()
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["errors"], 0, "{report}");
    assert_eq!(report["sessions"], 40, "{report}");
    assert_eq!(report["turns_completed"], 80, "{report}");

    // Each feature's sessions are reported on their own, and they did
    // what their feature does.
    let scenarios = &report["scenarios"];
    for (feature, sessions) in [
        ("files", 6),
        ("schedule", 4),
        ("team", 4),
        ("mcp", 4),
        ("sandbox", 2),
    ] {
        assert_eq!(
            scenarios[format!("feature:{feature}")]["count"],
            sessions * 2,
            "{feature}: {report}"
        );
    }

    // A turn finishes even when its tool fails, so read what each answered.
    let ids = std::fs::read_to_string(&sessions_file).unwrap();
    let _ = std::fs::remove_file(&sessions_file);
    for (i, id) in ids.lines().enumerate() {
        let said = answers(&fleet.events(id).await);
        let (first, last) = (said[0].as_str(), said[said.len() - 1].as_str());
        match MIX[i % MIX.len()] {
            // The note written in turn 1 reads back in turn 2.
            "files" | "sandbox" => assert!(last.contains("note 1"), "{id}: {said:?}"),
            "team" => assert!(first.contains("The answer is 42."), "{id}: {said:?}"),
            "mcp" => assert!(
                last.contains("stub found term 2"),
                "{id}: {said:?}, stub requests {}",
                stub_mcp::CALLS.load(Ordering::Relaxed)
            ),
            "schedule" => assert!(!last.to_lowercase().contains("error"), "{id}: {said:?}"),
            _ => assert!(!said.is_empty(), "{id}"),
        }
    }
    assert!(
        stub_mcp::TOOL_CALLS.load(Ordering::Relaxed) >= mcp_calls + 8,
        "every mcp turn called the stub server"
    );
}
