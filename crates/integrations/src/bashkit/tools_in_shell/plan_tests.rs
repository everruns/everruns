//! `tools plan` lists a script's calls and their risk without running any.

use super::*;

#[tokio::test]
async fn a_plan_lists_each_call_with_its_risk_and_runs_nothing() {
    let policy = Arc::new(Policy {
        approval_for: Some("mcp_github__get_issue"),
        previews: true,
        ..Policy::default()
    });
    let context = context(Some(policy.clone()), true);
    let output = run(
        r#"tools plan <<'EOF'
tools read-notes
tools web-fetch repo=a/b
tools github get-issue '{"number": 7}'
n=$(cat n.txt)
tools github list-pulls number=$n
EOF"#,
        &context,
    )
    .await;

    assert_eq!(output["exit_code"], 0, "{output}");
    let plan: Value = serde_json::from_str(output["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(
        plan,
        json!({
            "calls": [
                {"tool": "tools read-notes", "input": {}, "risk": "read_only", "where": "script"},
                {"tool": "tools web-fetch", "input": {"repo": "a/b"}, "risk": "changes",
                 "where": "script"},
                {"tool": "tools github get-issue", "input": {"number": 7},
                 "risk": "needs_approval", "where": "script"},
                {"tool": "tools github list-pulls", "input": null,
                 "risk": "checked_at_run_time", "where": "script"},
            ],
            "complete": true,
        })
    );
    assert!(
        policy.after.lock().unwrap().is_empty(),
        "planning runs no tool"
    );
}

#[tokio::test]
async fn a_plan_says_when_it_cannot_see_everything() {
    let context = context(Some(Arc::new(Policy::default())), true);
    let output = run(
        r#"tools plan <<'EOF'
cmd=web-fetch
eval "tools $cmd repo=a/b"
EOF"#,
        &context,
    )
    .await;
    let plan: Value = serde_json::from_str(output["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(plan["complete"], false, "{plan}");
}
