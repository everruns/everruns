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

// The command-backed worker returns directory listings rather than SessionFile
// values from read_file. Empty directory existence must come from metadata.
#[tokio::test]
async fn recursive_copy_preserves_children_under_an_empty_directory() {
    let (context, _) = create_context_with_mock_store();
    let result = BashTool::default()
        .execute_with_context(
            json!({"commands": r#"
set -e
mkdir -p /workspace/source/alpha /workspace/source/beta /workspace/destination
printf first > /workspace/source/alpha/SKILL.md
printf second > /workspace/source/beta/SKILL.md
cp -r /workspace/source/* /workspace/destination/
cat /workspace/destination/alpha/SKILL.md
cat /workspace/destination/beta/SKILL.md
"#}),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("expected shell result, got {result:?}");
    };
    assert_eq!(output["exit_code"], 0, "{output:?}");
    assert_eq!(output["stdout"], "firstsecond");
    assert_eq!(output["stderr"], "");
}

#[tokio::test]
async fn recursive_copy_into_empty_directory_keeps_the_source_name() {
    let (context, _) = create_context_with_mock_store();
    let result = BashTool::default()
        .execute_with_context(
            json!({"commands": r#"
set -e
mkdir -p /workspace/source/references /workspace/destination
printf guide > /workspace/source/SKILL.md
printf detail > /workspace/source/references/details.md
cp -r /workspace/source /workspace/destination/
cat /workspace/destination/source/SKILL.md
cat /workspace/destination/source/references/details.md
"#}),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("expected shell result, got {result:?}");
    };
    assert_eq!(output["exit_code"], 0, "{output:?}");
    assert_eq!(output["stdout"], "guidedetail");
    assert_eq!(output["stderr"], "");
}

#[tokio::test]
async fn embedded_python_copies_files_into_an_empty_workspace_directory() {
    let (context, _) = create_context_with_mock_store();
    let result = BashTool::default()
        .execute_with_context(
            json!({"commands": r#"
set -e
mkdir -p /workspace/python-source /workspace/python-target
printf payload > /workspace/python-source/input.txt
python3 -c 'import shutil; shutil.copytree("/workspace/python-source", "/workspace/python-target", dirs_exist_ok=True)'
cat /workspace/python-target/input.txt
"#}),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("expected shell result, got {result:?}");
    };
    assert_eq!(output["exit_code"], 0, "{output:?}");
    assert_eq!(output["stdout"], "payload");
    assert_eq!(output["stderr"], "");
}
