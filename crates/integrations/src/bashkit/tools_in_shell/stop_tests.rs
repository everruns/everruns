//! A call that needs a person's approval stops the script and reports what ran.

use super::*;
use everruns_contracts::tool_approval_types::ToolApprovalRequired;

fn gated_context() -> (ToolContext, Arc<Policy>) {
    let policy = Arc::new(Policy {
        approval_for: Some("mcp_github__get_issue"),
        ..Policy::default()
    });
    (context(Some(policy.clone()), true), policy)
}

fn as_tool_result(output: &Value) -> ToolResult {
    ToolResult {
        tool_call_id: "call_outer".into(),
        result: Some(output.clone()),
        images: None,
        error: None,
        connection_required: None,
        raw_output: None,
    }
}

#[tokio::test]
async fn a_gated_call_stops_the_script_and_raises_the_approval() {
    let (context, policy) = gated_context();
    let output = run(
        r#"tools web-fetch repo=a/b > /dev/null
tools read-notes > /dev/null
tools github get-issue number=7
echo after"#,
        &context,
    )
    .await;

    assert!(
        !output["stdout"].as_str().unwrap().contains("after"),
        "nothing after the stop runs: {output}"
    );
    assert_eq!(output["success"], false, "{output}");
    let report = &output["tools"];
    assert_eq!(
        report["done"],
        json!([{"tool": "tools web-fetch", "input": {"repo": "a/b"}, "ok": true,
                "result": {"tool": "web_fetch", "input": {"repo": "a/b"}}}]),
        "the mutating call is listed with its outcome"
    );
    assert_eq!(report["read_only_calls"], 1, "read-only calls are counted");
    assert_eq!(
        report["stopped"],
        json!({"reason": "needs_approval", "approval": "requested",
               "call": {"tool": "tools github get-issue", "input": {"number": 7}}})
    );

    // The turn's pause hook reads the gate's payload off the `bash` result,
    // so the person is asked about the nested call itself.
    let request = ToolApprovalRequired::from_tool_result(&as_tool_result(&output))
        .expect("the bash result carries the approval request");
    assert_eq!(request.tool, "mcp_github__get_issue");
    assert_eq!(request.arguments, json!({"number": 7}));
    assert_eq!(
        policy.after.lock().unwrap().as_slice(),
        ["web_fetch", "read_notes"],
        "the gated call never ran"
    );
}

#[tokio::test]
async fn no_call_runs_after_a_stop_even_where_the_exit_does_not_reach() {
    let (context, policy) = gated_context();
    let output = run(
        r#"(tools github get-issue number=1); tools web-fetch; echo "code=$?""#,
        &context,
    )
    .await;
    assert!(
        policy.after.lock().unwrap().is_empty(),
        "the call after the stop was refused: {output}"
    );
    assert_eq!(output["tools"]["stopped"]["reason"], "needs_approval");
}

#[tokio::test]
async fn a_failed_script_reports_its_mutating_calls() {
    let (context, _) = gated_context();
    let output = run("tools web-fetch > /dev/null; exit 3", &context).await;
    assert_eq!(output["exit_code"], 3, "{output}");
    assert_eq!(output["tools"]["done"][0]["tool"], "tools web-fetch");
    assert!(output["tools"].get("stopped").is_none());
    assert!(ToolApprovalRequired::from_tool_result(&as_tool_result(&output)).is_none());
}

#[tokio::test]
async fn a_clean_script_carries_no_report() {
    let (context, _) = gated_context();
    let output = run("tools web-fetch > /dev/null", &context).await;
    assert_eq!(output["exit_code"], 0, "{output}");
    assert!(output.get("tools").is_none(), "{output}");
}

#[tokio::test]
async fn the_call_limit_stops_the_script_with_a_report() {
    let context = context(Some(Arc::new(Policy::default())), true);
    let script = format!(
        "for i in $(seq 1 {}); do tools web-fetch > /dev/null; done; echo after",
        builtin::MAX_CALLS_PER_EXECUTION + 5
    );
    let output = run(&script, &context).await;
    assert!(!output["stdout"].as_str().unwrap().contains("after"));
    assert_eq!(output["tools"]["stopped"]["reason"], "call_limit");
    assert_eq!(
        output["tools"]["done"].as_array().unwrap().len(),
        builtin::MAX_CALLS_PER_EXECUTION
    );
}
