use super::*;

/// A failed command exits non-zero, so `&&` and `set -e` behave, and the
/// message is the tool's own — it already decided what may reach the model.
#[tokio::test]
async fn a_failed_command_exits_non_zero_with_the_tools_message() {
    let (context, _) = context_with_spelled_tool(Some("unknown verb: lst".to_string()));
    let tool = BashTool::default();

    let result = tool
        .execute_with_context(
            json!({"commands": "everruns agents lst && echo unreachable"}),
            &context,
        )
        .await;

    let call = everruns_contracts::tool_types::ToolCall {
        id: "call_shell".into(),
        name: "bash".into(),
        arguments: json!({"commands": "everruns agents lst && echo unreachable"}),
    };
    let result = result.into_tool_result(&call.id, &call.name);
    let output = result
        .result
        .as_ref()
        .expect("the shell ran and returned output");
    let event = everruns_contracts::runtime::events::ToolCompletedData::from_tool_result(
        &call, &result, "error", None,
    );
    assert!(event.success);
    assert_eq!(event.status, "success");
    let outcome = event.command_outcome.expect("the shell exit is reported");
    assert_eq!(
        i64::from(outcome.exit_code),
        output["exit_code"].as_i64().unwrap()
    );
    assert!(!outcome.success);
    assert_ne!(output["exit_code"], 0);
    assert!(
        output["stderr"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown verb: lst"),
        "{output:?}"
    );
    assert!(
        !output["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("unreachable"),
        "a non-zero exit must short-circuit `&&`: {output:?}"
    );
}
