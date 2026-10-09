use super::*;

#[tokio::test]
async fn test_playground_binding_survives_postgres_projections_and_filters() {
    let backend = create_test_backend().await;
    let subject = backend
        .resolve_runtime_identity(
            everruns_server::storage::runtime_identity::VerifiedRuntimeIdentity {
                org_id: TEST_ORG_ID,
                provider: "playground-test".into(),
                realm: "test".into(),
                subject: Uuid::now_v7().to_string(),
                name: "Test subject".into(),
                avatar_url: None,
                management_user_id: None,
            },
        )
        .await
        .unwrap();
    let owner = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            harness_id: Some(ensure_test_harness_id(&backend).await),
            owner_principal_id: owner,
            source: everruns_server::domains::sessions::record::SessionSource::Playground,
            playground_user_id: Some(subject.id),
            ..base_session_row(TEST_ORG_ID)
        })
        .await
        .unwrap();
    assert_eq!(session.playground_user_id, Some(subject.id));
    assert_eq!(
        backend
            .get_session(TEST_ORG_ID, session.id)
            .await
            .unwrap()
            .unwrap()
            .playground_user_id,
        Some(subject.id)
    );
    assert_eq!(
        backend
            .get_session_unscoped(session.id)
            .await
            .unwrap()
            .unwrap()
            .playground_user_id,
        Some(subject.id)
    );
    let filters = SessionListFilters {
        playground_user_id: Some(subject.id),
        sources: vec![everruns_server::domains::sessions::record::SessionSource::Playground],
        ..Default::default()
    };
    let (rows, total) = backend
        .list_sessions(TEST_ORG_ID, &filters, Pagination::new(0, 20))
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].playground_user_id, Some(subject.id));
    backend
        .set_session_archived(TEST_ORG_ID, session.id, true)
        .await
        .unwrap();
    let (rows, total) = backend
        .list_sessions(TEST_ORG_ID, &filters, Pagination::new(0, 20))
        .await
        .unwrap();
    assert_eq!(total, 0);
    assert!(rows.is_empty());
    let (_, total) = backend
        .list_sessions(
            TEST_ORG_ID,
            &SessionListFilters {
                archived_only: true,
                ..filters
            },
            Pagination::new(0, 20),
        )
        .await
        .unwrap();
    assert_eq!(total, 1);
    backend
        .delete_session(TEST_ORG_ID, session.id)
        .await
        .unwrap();
}
