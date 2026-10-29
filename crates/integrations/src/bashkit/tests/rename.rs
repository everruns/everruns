use super::*;

#[tokio::test]
async fn shell_mv_and_python_rename_move_nonempty_directories() {
    let (context, _) = create_context_with_mock_store();
    let result = BashTool::default()
        .execute_with_context(
            json!({"commands": r#"
set -e
mkdir -p /workspace/staged/empty
printf guide > /workspace/staged/SKILL.md
mv /workspace/staged /workspace/moved
python3 -c 'import os; os.rename("/workspace/moved", "/workspace/installed")'
test ! -e /workspace/staged
test ! -e /workspace/moved
test -d /workspace/installed/empty
cat /workspace/installed/SKILL.md
"#}),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("expected shell result, got {result:?}");
    };
    assert_eq!(output["exit_code"], 0, "{output:?}");
    assert_eq!(output["stdout"], "guide");
    assert_eq!(output["stderr"], "");
}

pub(super) fn move_directory(
    store: &MockFileStore,
    session: SessionId,
    from: &str,
    to: &str,
) -> everruns_contracts::error::Result<()> {
    let mut files = store.files.lock().unwrap();
    let mut dirs = store.directories.lock().unwrap();
    let prefix = format!("{from}/");
    let keys: Vec<_> = files
        .keys()
        .filter(|(s, p)| *s == session && p.starts_with(&prefix))
        .cloned()
        .collect();
    for key in keys {
        let content = files.remove(&key).unwrap();
        files.insert((session, format!("{to}{}", &key.1[from.len()..])), content);
    }
    let keys: Vec<_> = dirs
        .keys()
        .filter(|(s, p)| *s == session && (p == from || p.starts_with(&prefix)))
        .cloned()
        .collect();
    for key in keys {
        let metadata = dirs.remove(&key).unwrap();
        dirs.insert((session, format!("{to}{}", &key.1[from.len()..])), metadata);
    }
    Ok(())
}

#[tokio::test]
async fn directory_rename_preserves_nested_binary_files_and_empty_directories() {
    let session = SessionId::new();
    let store = Arc::new(MockFileStore::new());
    let adapter = SessionFileSystemAdapter::new(session, store);
    for path in [
        "/workspace/source",
        "/workspace/source/nested",
        "/workspace/source/nested/empty",
    ] {
        adapter.mkdir(Path::new(path), true).await.unwrap();
    }
    adapter
        .write_file(
            Path::new("/workspace/source/nested/data.bin"),
            &[0, 255, 17],
        )
        .await
        .unwrap();
    adapter
        .rename(
            Path::new("/workspace/source"),
            Path::new("/workspace/destination"),
        )
        .await
        .unwrap();
    assert!(
        !adapter
            .exists(Path::new("/workspace/source"))
            .await
            .unwrap()
    );
    assert!(
        adapter
            .stat(Path::new("/workspace/destination/nested/empty"))
            .await
            .unwrap()
            .file_type
            == FileType::Directory
    );
    assert_eq!(
        adapter
            .read_file(Path::new("/workspace/destination/nested/data.bin"))
            .await
            .unwrap(),
        [0, 255, 17]
    );
}

#[tokio::test]
async fn test_adapter_rename_file() {
    let session_id = SessionId::new();
    let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
    let adapter = SessionFileSystemAdapter::new(session_id, store);

    // Write original file
    adapter
        .write_file(Path::new("/workspace/old.txt"), b"data")
        .await
        .unwrap();

    // Rename it
    adapter
        .rename(
            Path::new("/workspace/old.txt"),
            Path::new("/workspace/new.txt"),
        )
        .await
        .unwrap();

    // Old file should not exist
    let old_result = adapter.read_file(Path::new("/workspace/old.txt")).await;
    assert!(old_result.is_err());

    // New file should have the content
    let new_content = adapter
        .read_file(Path::new("/workspace/new.txt"))
        .await
        .unwrap();
    assert_eq!(new_content, b"data");
}
