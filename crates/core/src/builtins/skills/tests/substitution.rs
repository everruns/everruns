use super::*;

// ========================================================================
// activate_skill env var substitution tests
// ========================================================================

#[tokio::test]
async fn test_activate_skill_substitutes_both_env_vars() {
    let fs = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    let skill_md = "---\nname: test-both\ndescription: Both vars.\n---\n\n${SKILL_DIR}/run.sh --session ${SESSION_ID}";
    fs.add_file(session_id, "/.agents/skills/test-both/SKILL.md", skill_md);

    let context = ToolContext::with_file_store(session_id, fs);
    let tool = ActivateSkillFromVfsTool;

    let result = tool
        .execute_with_context(serde_json::json!({"name": "test-both"}), &context)
        .await;
    match result {
        ToolExecutionResult::Success(val) => {
            let instructions = val["instructions"].as_str().unwrap();
            let expected_id = session_id.to_string();
            assert!(instructions.contains("/.agents/skills/test-both/run.sh"));
            assert!(instructions.contains(&format!("--session {}", expected_id)));
            assert!(!instructions.contains("${SESSION_ID}"));
            assert!(!instructions.contains("${SKILL_DIR}"));
        }
        other => panic!("Expected Success, got: {:?}", other),
    }
}

// Regression test for PR #1449 / EVE-388: SKILL.md written via the VFS
// at runtime must leave `!`command`` placeholders literal — no shell
// execution on the worker host.
#[tokio::test]
async fn test_activate_skill_does_not_execute_commands_for_writable_skill() {
    let fs = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    let skill_md = "---\nname: test-no-exec\ndescription: Leaves command placeholders literal.\n---\n\nLiteral: !`echo pwned`";
    // add_file — writable (user-authored) source.
    fs.add_file(
        session_id,
        "/.agents/skills/test-no-exec/SKILL.md",
        skill_md,
    );

    let context = ToolContext::with_file_store(session_id, fs);
    let tool = ActivateSkillFromVfsTool;

    let result = tool
        .execute_with_context(serde_json::json!({"name": "test-no-exec"}), &context)
        .await;
    match result {
        ToolExecutionResult::Success(val) => {
            let instructions = val["instructions"].as_str().unwrap();
            assert!(
                instructions.contains("Literal: !`echo pwned`"),
                "writable SKILL.md must not execute commands; got: {}",
                instructions
            );
            assert!(
                !instructions.contains("pwned\n") && !instructions.contains("\npwned"),
                "command output must not appear in skill instructions; got: {}",
                instructions
            );
        }
        other => panic!("Expected Success, got: {:?}", other),
    }
}

// Regression test for the Copilot review of EVE-388: a SKILL.md whose
// `is_readonly = true` does NOT earn trust. Users can set `is_readonly`
// via the session-files API and `InitialFile`, so it is not a valid
// provenance signal. The trust gate must remain off until a
// platform-controlled signal (e.g. `mount_capability_id`) exists.
#[tokio::test]
async fn test_activate_skill_does_not_execute_commands_for_readonly_user_skill() {
    let fs = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    let skill_md = "---\nname: test-readonly-no-exec\ndescription: is_readonly alone must not unlock exec.\n---\n\nLiteral: !`echo pwned`";
    // add_readonly_file simulates a user marking a SKILL.md is_readonly=true
    // via the session-files API or InitialFile configuration.
    fs.add_readonly_file(
        session_id,
        "/.agents/skills/test-readonly-no-exec/SKILL.md",
        skill_md,
    );

    let context = ToolContext::with_file_store(session_id, fs);
    let tool = ActivateSkillFromVfsTool;

    let result = tool
        .execute_with_context(
            serde_json::json!({"name": "test-readonly-no-exec"}),
            &context,
        )
        .await;
    match result {
        ToolExecutionResult::Success(val) => {
            let instructions = val["instructions"].as_str().unwrap();
            assert!(
                instructions.contains("Literal: !`echo pwned`"),
                "is_readonly=true must not bypass the command-substitution gate; got: {}",
                instructions
            );
            assert!(
                !instructions.contains("pwned\n") && !instructions.contains("\npwned"),
                "command output must not appear in skill instructions; got: {}",
                instructions
            );
        }
        other => panic!("Expected Success, got: {:?}", other),
    }
}

#[tokio::test]
async fn activation_preserves_literal_and_empty_arguments_through_vfs_pipeline() {
    let fs = Arc::new(MockFileStore::new());
    let session_id = SessionId::new();
    fs.add_file(session_id,"/.agents/skills/args/SKILL.md","---\nname: args\ndescription: Literal arguments.\n---\n$ARGUMENTS[0]|$1|$2|${SKILL_DIR}|!`unused-command`");
    let context = ToolContext::with_file_store(session_id, fs);
    let result = ActivateSkillFromVfsTool
        .execute_with_context(
            serde_json::json!({"name":"args","arguments":"'$1' \"\" tail"}),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success: {result:?}")
    };
    assert_eq!(
        value,
        serde_json::json!({"skill":"args","description":"Literal arguments.","instructions":"<skill name=\"args\">\n$1||tail|/.agents/skills/args|!`unused-command`\n</skill>"})
    );
}
