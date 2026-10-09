use super::*;

#[tokio::test]
async fn combined_agent_read_preserves_projection_lifecycle_and_org_scope() {
    use crate::storage::models::UpdateAgent;
    use everruns_core::DependencyBlocker;

    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let id = seed_agent(&adapters.db).await;
    let (definition, blocker) = adapters.resolve_agent_read(org_id, id).await.unwrap();
    assert!(blocker.is_none());
    assert_eq!(
        serde_json::to_value(definition.unwrap()).unwrap(),
        serde_json::to_value(adapters.get_agent(org_id, id).await.unwrap()).unwrap(),
    );

    for (status, expected) in [
        ("archived", DependencyBlocker::AgentArchived),
        ("deleted", DependencyBlocker::AgentDeleted),
    ] {
        adapters
            .db
            .update_agent(
                org_id,
                AgentId::from_uuid(id),
                UpdateAgent {
                    status: Some(status.to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        let (definition, blocker) = adapters.resolve_agent_read(org_id, id).await.unwrap();
        assert_eq!(blocker.unwrap().message(), expected.message());
        assert_eq!(
            definition.unwrap_err().to_string(),
            adapters
                .get_agent(org_id, id)
                .await
                .unwrap_err()
                .to_string(),
        );
    }

    for (org, lookup_id) in [(org_id, Uuid::new_v4()), (org_id + 1, id)] {
        let (definition, blocker) = adapters.resolve_agent_read(org, lookup_id).await.unwrap();
        assert!(definition.unwrap().is_none());
        assert!(matches!(blocker, Some(DependencyBlocker::AgentDeleted)));
    }
}

#[tokio::test]
async fn combined_harness_read_preserves_inheritance_lifecycle_and_org_scope() {
    use crate::storage::models::UpdateHarness;
    use everruns_core::DependencyBlocker;

    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let parent = seed_harness_for_platform_store(&adapters.db, org_id, "parent", false).await;
    let child = seed_harness_for_platform_store(&adapters.db, org_id, "child", false).await;
    adapters
        .db
        .update_harness(
            org_id,
            child,
            UpdateHarness {
                parent_harness_id: Some(Some(parent)),
                system_prompt: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let (definition, blocker) = adapters
        .resolve_harness_read(org_id, child.uuid())
        .await
        .unwrap();
    assert!(blocker.is_none());
    let definition = definition.unwrap().unwrap();
    assert_eq!(definition.system_prompt.as_deref(), Some("test prompt"));
    assert_eq!(
        serde_json::to_value(definition).unwrap(),
        serde_json::to_value(
            adapters
                .get_harness(org_id, child.uuid())
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
    );

    for (status, expected) in [
        ("archived", DependencyBlocker::HarnessArchived),
        ("deleted", DependencyBlocker::HarnessDeleted),
    ] {
        adapters
            .db
            .update_harness(
                org_id,
                child,
                UpdateHarness {
                    status: Some(status.to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        let (definition, blocker) = adapters
            .resolve_harness_read(org_id, child.uuid())
            .await
            .unwrap();
        assert_eq!(blocker.unwrap().message(), expected.message());
        assert_eq!(
            definition.unwrap_err().to_string(),
            adapters
                .get_harness(org_id, child.uuid())
                .await
                .unwrap_err()
                .to_string(),
        );
    }

    for (org, id) in [(org_id, Uuid::new_v4()), (org_id + 1, child.uuid())] {
        let (definition, blocker) = adapters.resolve_harness_read(org, id).await.unwrap();
        assert!(definition.unwrap().is_none());
        assert!(matches!(blocker, Some(DependencyBlocker::HarnessDeleted)));
    }
}
