use super::*;

#[tokio::test]
async fn test_bash_python3_runs_cpython() {
    let (context, _) = create_context_with_mock_store();
    let tool = BashTool::default();

    let result = tool
        .execute_with_context(
            json!({"commands": "python3 -c \"import sys; print(sys.implementation.name)\""}),
            &context,
        )
        .await;

    match result {
        ToolExecutionResult::Success(output) => {
            assert_eq!(
                output["stdout"], "cpython\n",
                "unexpected output: {:?}",
                output
            );
        }
        other => panic!("expected success, got: {:?}", other),
    }
}
