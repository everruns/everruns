//! Review commands use the same complete live Memory filesystem as remote workers.
use super::*;

#[tokio::test]
async fn accepted_dreaming_artifact_and_edits_share_live_memory() {
    let adapters = test_adapters();
    let org = everruns_core::DEFAULT_ORG_ID;
    let harness = seed_harness_for_platform_store(&adapters.db, org, "platform-chat", true).await;
    let session = seed_platform_session(&adapters.db, org, harness, None).await;
    let host = everruns_worker::WorkerRuntimeHost::new(adapters.clone());
    let files = everruns_core::host::RuntimeHostAdapter::file_store(&host, org);
    let root = "/memory/shared/dreaming-review";
    let path = format!("{root}/dream-operation-0.md");
    assert!(files.stat_file(session, root).await.unwrap().is_none());
    files.create_directory(session, root).await.unwrap();
    assert!(
        files
            .create_file_if_absent(session, &path, "approved preference")
            .await
            .unwrap()
    );
    let entries = files.list_directory(session, root).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, path);
    assert_eq!(
        files
            .read_file(session, &path)
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("approved preference")
    );

    // A partial read-only adapter override would leave this edit in a workspace
    // snapshot and make the runtime keep reading the obsolete Memory artifact.
    files
        .write_file(session, &path, "edited preference", "text")
        .await
        .unwrap();
    assert!(
        !files
            .create_file_if_absent(session, &path, "replacement")
            .await
            .unwrap()
    );
    assert_eq!(
        files
            .read_file(session, &path)
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("edited preference")
    );
    assert!(
        files
            .write_file_if_content_matches(session, &path, "obsolete", "text", "wrong", "text")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        files
            .write_file_if_content_matches(
                session,
                &path,
                "edited preference",
                "text",
                "final preference",
                "text"
            )
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        files
            .read_file(session, &path)
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("final preference")
    );
    assert!(files.delete_file(session, &path, false).await.unwrap());
    assert!(files.read_file(session, &path).await.unwrap().is_none());
    assert!(
        adapters
            .db
            .get_session_file(session.uuid(), root)
            .await
            .unwrap()
            .is_none(),
        "Memory directories must never be copied into workspace storage"
    );
    assert!(
        adapters
            .db
            .get_session_file(session.uuid(), &path)
            .await
            .unwrap()
            .is_none(),
        "Memory edits must never be copied into workspace storage"
    );
}

#[tokio::test]
async fn dreaming_adoption_is_live_on_the_direct_loading_seam() {
    let adapters = test_adapters();
    let org = everruns_core::DEFAULT_ORG_ID;
    for enabled in [false, true, false] {
        adapters
            .db
            .replace_org_feature_flags(
                org,
                &std::collections::HashMap::from([("dreaming".to_string(), enabled)]),
            )
            .await
            .unwrap();
        let mut capabilities = vec![
            AgentCapabilityConfig::new("dreaming"),
            AgentCapabilityConfig::new("session_file_system"),
        ];
        adapters
            .filter_dreaming(org, &mut capabilities)
            .await
            .unwrap();
        assert_eq!(
            capabilities.iter().any(|c| c.capability_id() == "dreaming"),
            enabled
        );
        assert!(
            capabilities
                .iter()
                .any(|c| c.capability_id() == "session_file_system")
        );
    }
}

#[tokio::test]
async fn due_dreaming_discovery_ignores_revoked_organizations() {
    use everruns_worker::internal_commands::InternalCommandTransport;
    let adapters = test_adapters();
    let org = everruns_core::DEFAULT_ORG_ID;
    let harness = seed_harness_for_platform_store(&adapters.db, org, "dreaming-due", true).await;
    let session = seed_platform_session(&adapters.db, org, harness, None).await;
    sqlx::query("INSERT INTO dreaming_state(org_id,key,revision,value_encrypted,session_id,due_at) VALUES($1,'job:due',1,$2,$3,0)")
        .bind(org).bind(vec![0u8]).bind(session.uuid()).execute(adapters.db.pool()).await.unwrap();
    for enabled in [false, true, false] {
        adapters
            .db
            .replace_org_feature_flags(
                org,
                &std::collections::HashMap::from([("dreaming".to_string(), enabled)]),
            )
            .await
            .unwrap();
        let due = adapters
            .internal_commands(org)
            .execute_internal_command(
                "worker_dreaming_state",
                serde_json::json!({"operation":"due","now":1,"limit":1}),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(due.as_array().unwrap().len(), usize::from(enabled));
    }
}
