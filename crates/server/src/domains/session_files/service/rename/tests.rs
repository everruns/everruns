use super::*;

#[tokio::test]
async fn directory_rename_cannot_relocate_or_replace_nested_virtual_mounts() {
    use crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry;
    use everruns_core::capability_types::VirtualFileTree;
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let registry = Arc::new(VirtualMountRegistry::new());
    let service = WorkspaceFileService::new(db.clone()).with_virtual_registry(registry.clone());
    seed(&db, session, "/source", true, false).await;
    seed(&db, session, "/nested", true, false).await;
    let mut tree = VirtualFileTree::new();
    tree.insert_text("/nested/docs/readme", "readonly");
    registry.register(
        session.uuid(),
        "/nested/docs".into(),
        Arc::new(tree),
        "fixture".into(),
    );
    for (src, dst) in [("/nested", "/new"), ("/source", "/nested")] {
        let error = service
            .move_file(
                session.uuid(),
                MoveFileInput {
                    src_path: src.into(),
                    dst_path: dst.into(),
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("virtual mounts"), "{error}");
        assert!(
            db.get_session_file_info(session.uuid(), src)
                .await
                .unwrap()
                .is_some()
        );
    }
    assert!(
        db.get_session_file_info(session.uuid(), "/new")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        service
            .read_file(session.uuid(), "/nested/docs/readme")
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("readonly")
    );
}

pub(super) async fn seed(
    db: &StorageBackend,
    session: SessionId,
    path: &str,
    directory: bool,
    readonly: bool,
) -> SessionFileRow {
    db.create_session_file(CreateSessionFileRow {
        session_id: session,
        path: path.into(),
        content: (!directory).then(|| vec![0, 255, 17]),
        is_directory: directory,
        is_readonly: readonly,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn directory_rename_preserves_identity_and_literal_unicode_prefixes() {
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let service = WorkspaceFileService::new(db.clone());
    seed(&db, session, "/é_%", true, false).await;
    seed(&db, session, "/é_%/empty", true, false).await;
    let file = seed(&db, session, "/é_%/binary", false, false).await;
    let sibling = seed(&db, session, "/é_X/binary", false, false).await;
    SessionFileSystem::rename_directory(&service, session, "/é_%", "/renamed")
        .await
        .unwrap();
    assert!(
        db.get_session_file_info(session.uuid(), "/é_%")
            .await
            .unwrap()
            .is_none()
    );
    let moved = db
        .get_session_file(session.uuid(), "/renamed/binary")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.id, file.id);
    assert_eq!(moved.content, file.content);
    assert!(
        db.get_session_file_info(session.uuid(), "/renamed/empty")
            .await
            .unwrap()
            .unwrap()
            .is_directory
    );
    assert_eq!(
        db.get_session_file_info(session.uuid(), "/é_X/binary")
            .await
            .unwrap()
            .unwrap()
            .id,
        sibling.id
    );
}

#[tokio::test]
async fn directory_rename_rejects_collisions_protection_and_invalid_destinations() {
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let service = WorkspaceFileService::new(db.clone());
    seed(&db, session, "/source", true, false).await;
    let file = seed(&db, session, "/source/file", false, false).await;
    seed(&db, session, "/occupied", true, false).await;
    seed(&db, session, "/implicit/file", false, false).await;
    seed(&db, session, "/protected", true, true).await;
    for target in [
        "/",
        "/source",
        "/source/child",
        "/occupied",
        "/implicit",
        "/protected/child",
        "/knowledge/new",
        "/memory/user/new",
    ] {
        assert!(
            SessionFileSystem::rename_directory(&service, session, "/source", target)
                .await
                .is_err(),
            "{target}"
        );
        assert_eq!(
            db.get_session_file_info(session.uuid(), "/source/file")
                .await
                .unwrap()
                .unwrap()
                .id,
            file.id
        );
    }
    seed(&db, session, "/source/readonly", false, true).await;
    assert!(
        SessionFileSystem::rename_directory(&service, session, "/source", "/new")
            .await
            .is_err()
    );
    assert!(
        db.get_session_file_info(session.uuid(), "/new")
            .await
            .unwrap()
            .is_none()
    );
}
