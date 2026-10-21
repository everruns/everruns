//! Script and embedded-interpreter compatibility at the Framework boundary.

use super::*;

#[tokio::test]
async fn test_bash_execute_script_with_exit_code() {
    let (context, _) = create_context_with_mock_store();
    let tool = BashTool::default();

    // Create a script that exits with a specific code
    let result = tool
        .execute_with_context(
            json!({"commands": "cat > /workspace/fail.sh << 'EOF'\n#!/bin/bash\necho failing\nexit 42\nEOF"}),
            &context,
        )
        .await;
    assert!(matches!(result, ToolExecutionResult::Success(_)));

    // Execute and check exit code propagation
    let result = tool
        .execute_with_context(
            json!({"commands": "/workspace/fail.sh; echo \"code: $?\""}),
            &context,
        )
        .await;

    if let ToolExecutionResult::Success(output) = result {
        let stdout = output["stdout"].as_str().unwrap_or("");
        assert!(stdout.contains("failing"), "Script should have run");
        assert!(
            stdout.contains("code: 42"),
            "Exit code should propagate, got: {}",
            stdout
        );
    } else {
        panic!("Expected success, got: {:?}", result);
    }
}

#[tokio::test]
async fn embedded_python_runtime_executes_through_the_bash_tool() {
    let (context, _) = create_context_with_mock_store();
    let result = BashTool::default()
        .execute_with_context(json!({"commands": "python3 -c 'print(6 * 7)'"}), &context)
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("expected the embedded runtime to execute, got {result:?}");
    };
    assert_eq!(output["exit_code"], 0, "{output:?}");
    assert_eq!(output["stdout"], "42\n");
}
