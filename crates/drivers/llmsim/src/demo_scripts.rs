//! Pre-baked scripted configs for end-to-end demos that run without an LLM
//! API key (`LLMSIM_DEMO=...`).

use crate::{LlmSimConfig, SimToolCall, SimTurn};

/// Scripted multi-turn config that drives the Cloud Cost & Security Auditor
/// example agent through a small, deterministic AWS audit using only the
/// `fake_aws` capability's tools. Useful for the `user_hooks` end-to-end
/// demo (and any operator who wants to exercise the auditor without an LLM
/// API key).
///
/// Sequence of assistant turns:
///   1. Call `aws_list_ec2_instances`.
///   2. Call `aws_list_s3_buckets`.
///   3. Write a short audit summary as plain text.
///
/// After turn 3 the script repeats the final turn, matching the default
/// `OnExhausted::RepeatLast` behavior.
pub fn auditor_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Starting the audit. Listing EC2 instances first.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "aws_list_ec2_instances".to_string(),
                arguments: serde_json::json!({}),
                id: Some("call_demo_ec2".to_string()),
            }],
        },
        SimTurn::Mixed {
            text: "EC2 inventory captured. Listing S3 buckets next.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "aws_list_s3_buckets".to_string(),
                arguments: serde_json::json!({}),
                id: Some("call_demo_s3".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Audit complete: inventoried EC2 instances and S3 buckets. \
             See /workspace/.audit.log for the per-tool-call audit trail \
             written by the post_tool_use hook bundle."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}

/// Scripted scenario that exercises the `pre_tool_use` block path. The
/// scripted agent first issues a destructive `bash` call (`rm -rf /`)
/// followed by a benign one (`ls -la`). When combined with a `pre_tool_use`
/// hook bundle that denies `rm -rf` patterns, the first tool call gets
/// blocked (the tool is not invoked) and the second succeeds — the agent
/// observes the difference in tool results.
///
/// Used by `LLMSIM_DEMO=guarded` to demonstrate `pre_tool_use` without an
/// LLM API key.
pub fn guarded_bash_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Step 1: attempting a destructive command.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "bash".to_string(),
                arguments: serde_json::json!({ "commands": "rm -rf /" }),
                id: Some("call_demo_rm".to_string()),
            }],
        },
        SimTurn::Mixed {
            text: "Step 2: trying a safe command.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "bash".to_string(),
                arguments: serde_json::json!({ "commands": "ls -la /workspace" }),
                id: Some("call_demo_ls".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Guarded-bash demo complete. The first tool call should be \
             blocked by the pre_tool_use hook; the second should succeed."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}

/// Scripted scenario that exercises the session task registry end-to-end
/// without an LLM API key. The scripted agent starts a background bash run
/// via `spawn_background` (which creates a `background_tool` session task),
/// then inspects the registry with `list_tasks`.
///
/// Used by `LLMSIM_DEMO=tasks`. Requires an agent with the `bashkit_shell`
/// and `session_tasks` capabilities.
pub fn session_tasks_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Kicking off a background bash run.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "spawn_background".to_string(),
                arguments: serde_json::json!({
                    "tool": "bash",
                    "args": { "commands": "echo task demo start; echo task demo done" },
                    "title": "Demo background run",
                    "signal_on_completion": false,
                }),
                id: Some("call_demo_spawn".to_string()),
            }],
        },
        SimTurn::Mixed {
            text: "Checking the session task registry.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "list_tasks".to_string(),
                arguments: serde_json::json!({}),
                id: Some("call_demo_list".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Session tasks demo complete: a background run was started and \
             tracked as a session task. Inspect it via \
             GET /v1/sessions/{session_id}/tasks."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}

/// Scripted scenario for the monitor task kind: spawns a recurring scheduled
/// monitor (cron fires at second 0 every minute) so the session scheduler
/// creates the schedule and a linked `monitor` task. Used by
/// `LLMSIM_DEMO=monitor` for end-to-end verification without an LLM API key.
pub fn monitor_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Setting up a recurring monitor.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "spawn_background".to_string(),
                arguments: serde_json::json!({
                    "tool": "bash",
                    "args": { "commands": "echo monitor check" },
                    "title": "Demo monitor",
                    "signal_on_completion": false,
                    "schedule": { "cron_expression": "0 * * * * * *", "timezone": "UTC" },
                }),
                id: Some("call_demo_monitor".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Monitor demo complete: a recurring monitor was scheduled and tracked as a session task. Inspect it via GET /v1/sessions/{session_id}/tasks."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}
