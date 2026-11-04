use super::*;

#[tokio::test]
async fn rename_cannot_cross_scope_even_when_workspace_and_session_keys_match() {
    let backend = Arc::new(FlatStore::default());
    let scoped = crate::runtime::session_files::WorkspaceScopedFileSystem::wrap(
        backend.clone(),
        crate::runtime::typed_id::WorkspaceId::from_uuid(sid().uuid()),
    );
    assert!(
        scoped
            .rename_directory(sid(), "/source", "/knowledge/destination")
            .await
            .is_err()
    );
    assert!(
        scoped
            .rename_directory(sid(), "/knowledge/source", "/destination")
            .await
            .is_err()
    );
    assert!(backend.renames.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rename_routes_workspace_aliases_but_never_moves_mounts() {
    let backend = Arc::new(FlatStore::default());
    let other = Arc::new(FlatStore::default());
    let fs =
        MountFs::new(backend.clone()).with_mount("/workspace/volumes/readonly", other.clone(), "/");
    fs.rename_directory(sid(), "/workspace/source", "/destination")
        .await
        .unwrap();
    assert_eq!(
        *backend.renames.lock().unwrap(),
        [("/source".into(), "/destination".into())]
    );
    for (from, to) in [
        ("/workspace", "/new"),
        ("/source", "/workspace"),
        ("/workspace/volumes", "/new"),
        ("/volumes", "/new"),
        ("/source", "/volumes"),
        ("/source", "/workspace/volumes/readonly/new"),
        ("/workspace/volumes/readonly", "/new"),
    ] {
        assert!(
            fs.rename_directory(sid(), from, to).await.is_err(),
            "{from} -> {to}"
        );
    }
    assert_eq!(backend.renames.lock().unwrap().len(), 1);
    assert!(other.renames.lock().unwrap().is_empty());
}
