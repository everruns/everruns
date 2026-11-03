use super::*;

#[tokio::test]
async fn test_adapter_answers_stat_from_its_listing_until_a_write() {
    let session_id = SessionId::new();
    let mock = Arc::new(MockFileStore::new());
    let store: Arc<dyn SessionFileSystem> = mock.clone();
    for name in ["a.rs", "b.rs", "c.rs"] {
        store
            .write_file(session_id, &format!("/workspace/src/{name}"), "x", "text")
            .await
            .unwrap();
    }
    let adapter = SessionFileSystemAdapter::new(session_id, store);
    let listed = adapter.read_dir(Path::new("/workspace/src")).await.unwrap();
    assert_eq!(listed.len(), 3);
    let before = mock.stat_calls.load(Ordering::Relaxed);
    for name in ["a.rs", "b.rs", "c.rs"] {
        let path = PathBuf::from(format!("/workspace/src/{name}"));
        assert!(adapter.stat(&path).await.unwrap().file_type.is_file());
        assert!(adapter.exists(&path).await.unwrap());
    }
    assert_eq!(
        mock.stat_calls.load(Ordering::Relaxed),
        before,
        "listed entries need no store round trip"
    );

    adapter
        .remove(Path::new("/workspace/src/a.rs"), false)
        .await
        .unwrap();
    assert!(
        !adapter
            .exists(Path::new("/workspace/src/a.rs"))
            .await
            .unwrap(),
        "a change through the adapter drops what it remembered"
    );
}

#[tokio::test]
async fn test_recursive_find_fetches_the_subtree_once() {
    let session_id = SessionId::new();
    let mock = Arc::new(MockFileStore::new());
    // Through MountFs, as production wires it.
    let store = crate::bashkit::mount_fs::MountFs::wrap(mock.clone());
    let mut expected = Vec::new();
    for dir in ["a", "a/x", "b", "b/y/z"] {
        for file in ["1.rs", "2.rs"] {
            let path = format!("/workspace/repo/{dir}/{file}");
            store
                .write_file(session_id, &path, "x", "text")
                .await
                .unwrap();
            expected.push(path);
        }
    }
    expected.sort();
    let mut context = ToolContext::new(session_id);
    context.file_store = Some(store);
    let before = (
        mock.list_calls.load(Ordering::Relaxed),
        mock.stat_calls.load(Ordering::Relaxed),
    );
    let result = BashTool::default()
        .execute_with_context(
            json!({"commands": "find /workspace/repo -type f | sort"}),
            &context,
        )
        .await;
    let ToolExecutionResult::Success(output) = result else {
        panic!("find failed: {result:?}");
    };
    let stdout = output["stdout"].as_str().unwrap_or_default();
    let found: Vec<&str> = stdout.lines().collect();
    assert_eq!(found, expected);
    assert_eq!(mock.tree_calls.load(Ordering::Relaxed), 1);
    let listings = mock.list_calls.load(Ordering::Relaxed) - before.0;
    let stats = mock.stat_calls.load(Ordering::Relaxed) - before.1;
    // Bashkit stats the start path and lists it; everything below comes
    // from the one subtree fetch (14 entries would otherwise cost ~20 calls).
    assert!(listings <= 1, "listings: {listings}");
    assert!(stats <= 2, "stats: {stats}");
}

/// Every file below `path`, relative to it, as a store with `list_tree` returns them.
pub(super) fn mock_tree(store: &MockFileStore, session_id: SessionId, path: &str) -> Vec<FileInfo> {
    store.tree_calls.fetch_add(1, Ordering::Relaxed);
    let prefix = format!(
        "{}/",
        MockFileStore::normalize_path(path).trim_end_matches('/')
    );
    let files = store.files.lock().unwrap();
    files
        .iter()
        .filter(|((sid, p), _)| *sid == session_id && p.starts_with(&prefix))
        .map(|((_, p), (content, _))| FileInfo {
            id: uuid::Uuid::new_v4(),
            session_id: session_id.into(),
            path: p[prefix.len()..].to_string(),
            name: p.rsplit('/').next().unwrap_or("").to_string(),
            is_directory: false,
            is_readonly: false,
            size_bytes: content.len() as i64,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .collect()
}
