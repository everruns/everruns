use super::*;

#[tokio::test]
async fn test_edit_file_failure_diagnostics_leave_batch_unchanged() {
    let store = Arc::new(MockFileStore::default());
    let content = "header\nlet enabled = false;\nfooter\n";
    store.add_text_file("/hints.txt", content);
    let context = make_context(store.clone());
    let expected_hash = read_hash(&context, "/workspace/hints.txt").await;
    let error = expect_tool_error(
        EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/hints.txt",
                    "expected_hash": expected_hash,
                    "edits": [
                        {"old_text": "header", "new_text": "let enabled = true;"},
                        {"old_text": "let enabled=false;", "new_text": "let enabled = true;"}
                    ]
                }),
                &context,
            )
            .await,
    );
    assert!(error.contains("Edit 2"), "{error}");
    assert!(error.contains("2: \"let enabled = false;\""), "{error}");
    assert!(error.contains("No edits were applied"), "{error}");
    assert!(!error.contains("may already be applied"), "{error}");
    assert_eq!(store.content("/hints.txt").unwrap(), content);
}

#[tokio::test]
async fn test_edit_file_diagnostics_real_disk_recovery_flow() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(everruns_core::host::RealDiskFileStore::new(directory.path()).unwrap());
    let context =
        ToolContext::with_file_store(SessionId::new(), self::mount_fs::MountFs::wrap(store));
    let written = expect_success(
        WriteFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/settings.rs",
                    "content": "header\nlet enabled = false;\n"
                }),
                &context,
            )
            .await,
    );
    let error = expect_tool_error(
        EditFileTool
            .execute_with_context(
                json!({
                    "path": "/workspace/settings.rs",
                    "expected_hash": written["content_hash"],
                    "edits": [
                        {"old_text": "header", "new_text": "updated header"},
                        {"old_text": "let enabled=false;", "new_text": "let enabled = true;"}
                    ]
                }),
                &context,
            )
            .await,
    );
    println!("Failed batch: {error}");
    assert!(error.contains("2: \"let enabled = false;\""));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("settings.rs")).unwrap(),
        "header\nlet enabled = false;\n"
    );
    let hash = read_hash(&context, "/workspace/settings.rs").await;
    expect_success(EditFileTool.execute_with_context(json!({
        "path": "/workspace/settings.rs",
        "expected_hash": hash,
        "edits": [{"old_text": "let enabled = false;", "new_text": "let enabled = true;"}]
    }), &context).await);
    let retry = expect_tool_error(EditFileTool.execute_with_context(json!({
        "path": "/workspace/settings.rs",
        "expected_hash": hash,
        "edits": [{"old_text": "let enabled = false;", "new_text": "let enabled = true;"}]
    }), &context).await);
    println!("Repeated edit: {retry}");
    assert!(retry.contains("changed since the last read"));
    assert!(retry.contains("may already be applied"));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("settings.rs")).unwrap(),
        "header\nlet enabled = true;\n"
    );
}

#[tokio::test]
async fn test_edit_file_diagnostics_do_not_bypass_read_policy() {
    let store = Arc::new(MockFileStore::default());
    store.add_text_file("/private.txt", "private content sentinel");
    let policy = everruns_core::WorkspacePolicy::builder()
        .allow_read("/")
        .allow_write("/")
        .deny_read("private.txt")
        .build()
        .unwrap();
    let guarded = Arc::new(everruns_core::host::PolicyFileStore::new(
        store.clone(),
        policy,
    ));
    let context =
        ToolContext::with_file_store(SessionId::new(), self::mount_fs::MountFs::wrap(guarded));
    let result = EditFileTool
        .execute_with_context(
            json!({
                "path": "/workspace/private.txt",
                "expected_hash": "stale",
                "edits": [{"old_text": "private content sentine", "new_text": "changed"}]
            }),
            &context,
        )
        .await;
    assert!(
        matches!(result, ToolExecutionResult::InternalError(_)),
        "{result:?}"
    );
    assert!(!format!("{result:?}").contains("private content sentinel"));
    assert_eq!(
        store.content("/private.txt").unwrap(),
        "private content sentinel"
    );
}

#[test]
fn test_edit_file_diagnostics_use_original_bom_and_crlf_snapshot() {
    for newline in ["\n", "\r\n", "\r"] {
        let content = format!("\u{feff}header{newline}let enabled = true;{newline}");
        let error = apply_text_edits(
            &content,
            &[
                TextEdit {
                    old_text: "header".into(),
                    new_text: "changed header".into(),
                },
                TextEdit {
                    old_text: "\u{feff}let enabled = false;\n".into(),
                    new_text: "\u{feff}let enabled = true;\n".into(),
                },
            ],
        )
        .unwrap_err();
        assert!(error.contains("Edit 2"), "{error}");
        assert!(error.contains("2: \"let enabled = true;\""), "{error}");
        assert!(
            error.contains("this edit may already be applied"),
            "{error}"
        );
        assert!(error.contains("No edits were applied"), "{error}");
    }
}

#[test]
fn test_edit_file_already_applied_is_only_a_hint() {
    let error = apply_text_edits(
        "let enabled = true;\n",
        &[TextEdit {
            old_text: "let enabled = false;\n".into(),
            new_text: "let enabled = true;\n".into(),
        }],
    )
    .unwrap_err();
    assert!(error.contains("may already be applied"), "{error}");
    assert!(error.contains("Verify"), "{error}");
    assert!(error.contains("No edits were applied"), "{error}");
}

#[tokio::test]
async fn test_edit_file_batch_replace_success() {
    let store = Arc::new(MockFileStore::default());
    store.add_text_file("/batch.txt", "one\ntwo\nthree\n");
    let context = make_context(store.clone());
    let expected_hash = read_hash(&context, "/workspace/batch.txt").await;

    let result = EditFileTool
        .execute_with_context(
            json!({
                "path": "/workspace/batch.txt",
                "expected_hash": expected_hash,
                "edits": [
                    {"old_text": "one", "new_text": "ONE"},
                    {"old_text": "three", "new_text": "THREE"}
                ]
            }),
            &context,
        )
        .await;
    let value = expect_success(result);

    assert_eq!(store.content("/batch.txt").unwrap(), "ONE\ntwo\nTHREE\n");
    assert_eq!(value["applied_edits"], 2);
    assert_eq!(value["rebased"], false);
    assert_eq!(value["first_changed_line"], 1);
}
