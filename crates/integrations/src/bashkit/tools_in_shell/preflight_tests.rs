//! A script whose visible `tools` call needs approval is held before it starts.

use super::*;
use everruns_contracts::tool_approval_types::ToolApprovalRequired;

fn previewing_context() -> (ToolContext, Arc<Policy>) {
    let policy = Arc::new(Policy {
        approval_for: Some("mcp_github__get_issue"),
        previews: true,
        ..Policy::default()
    });
    (context(Some(policy.clone()), true), policy)
}

fn approval_request(output: &Value) -> Option<ToolApprovalRequired> {
    ToolApprovalRequired::from_tool_result(&ToolResult {
        tool_call_id: "call_outer".into(),
        result: Some(output.clone()),
        images: None,
        error: None,
        connection_required: None,
        raw_output: None,
    })
}

#[tokio::test]
async fn a_visible_gated_call_holds_the_script_before_anything_runs() {
    let (context, policy) = previewing_context();
    let output = run(
        r#"echo started > started.txt
tools web-fetch repo=a/b
tools github get-issue '{"number": 7}'"#,
        &context,
    )
    .await;

    assert!(
        policy.after.lock().unwrap().is_empty(),
        "no call ran: {output}"
    );
    assert_eq!(output["success"], false);
    assert_eq!(output["tools"]["done"], json!([]));
    assert_eq!(
        output["tools"]["stopped"],
        json!({"reason": "needs_approval", "approval": "requested", "before_start": true,
               "call": {"tool": "tools github get-issue", "input": {"number": 7}}})
    );
    // The request binds to the exact call the script makes, so the one-off
    // answer lets that call through when the script runs again.
    let request = approval_request(&output).expect("the bash result asks");
    assert_eq!(request.tool, "mcp_github__get_issue");
    assert_eq!(request.arguments, json!({"number": 7}));

    let again = run("cat started.txt", &context).await;
    assert_ne!(again["exit_code"], 0, "the script's first line never ran");
}

#[tokio::test]
async fn calls_analysis_cannot_see_are_left_to_the_run() {
    let (context, policy) = previewing_context();
    // Built input, a function body, and stdin input are not previewed; the
    // run-time stop still catches the first that runs.
    for script in [
        r#"n=7; tools web-fetch; tools github get-issue number=$n"#,
        r#"get() { tools github get-issue number=7; }; tools web-fetch; get"#,
        r#"tools web-fetch; echo '{"number":7}' | tools github get-issue -"#,
    ] {
        policy.after.lock().unwrap().clear();
        let output = run(script, &context).await;
        assert_eq!(
            policy.after.lock().unwrap().as_slice(),
            ["web_fetch"],
            "the script started: {script} -> {output}"
        );
        assert!(output["tools"]["stopped"].get("before_start").is_none());
        assert_eq!(output["tools"]["stopped"]["reason"], "needs_approval");
    }
}

#[tokio::test]
async fn a_script_with_no_held_call_runs() {
    let (context, policy) = previewing_context();
    let output = run(
        "tools web-fetch > /dev/null; tools read-notes > /dev/null",
        &context,
    )
    .await;
    assert_eq!(output["exit_code"], 0, "{output}");
    assert_eq!(policy.after.lock().unwrap().len(), 2);
    assert!(approval_request(&output).is_none());
}
