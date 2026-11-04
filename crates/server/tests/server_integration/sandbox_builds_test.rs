//! Durable build queue contracts against real PostgreSQL. Provider qualification
//! separately exercises fresh compute, snapshots and cleanup against Daytona.
use chrono::Utc;
use everruns_contracts::{
    session_sandbox::SessionSandboxCredentialSource, typed_id::SandboxTemplateBuildId,
};
use everruns_server::{
    domains::{
        organizations::record::generate_org_public_id,
        sandbox_templates::{builds::record::*, record::*},
    },
    storage::{CreateOrganizationRow, StorageBackend},
};
use serde_json::json;

async fn setup() -> (StorageBackend, i64, SandboxTemplate) {
    let db = StorageBackend::test_database();
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: generate_org_public_id(),
            name: "Build tests".into(),
            created_by: None,
        })
        .await
        .unwrap()
        .org_id;
    let mut target = SandboxTargetSpec::managed("daytona");
    target.credential.source = SessionSandboxCredentialSource::Organization;
    let account = db
        .create_organization_connection(everruns_server::storage::CreateOrganizationConnectionRow {
            org_id: org,
            name: "Build account".into(),
            provider: "daytona".into(),
            access_token_encrypted: Some(vec![1, 2, 3]),
            ..Default::default()
        })
        .await
        .unwrap();
    target.credential.connection_id = Some(account.id);
    target.options = json!({"size":"small","workspace_path":"/home/daytona/workspace"});
    let template = db
        .create_sandbox_template(
            org,
            "builder",
            "Builder",
            None,
            &SandboxTemplateSpec {
                template_revision_id: None,
                target,
                containment: Some(SandboxContainmentSpec::isolated()),
                durability: None,
                lifecycle: SandboxLifecycle::default(),
                bootstrap: SandboxBootstrap {
                    commands: vec!["echo runtime-bootstrap".into()],
                },
            },
            false,
        )
        .await
        .unwrap();
    (db, org, template)
}
fn attempt(template: &SandboxTemplate) -> SandboxTemplateBuild {
    let now = Utc::now();
    let id = SandboxTemplateBuildId::new();
    SandboxTemplateBuild {
        id,
        sandbox_template_id: template.public_id,
        request: SandboxBuildRequest {
            source_revision_id: template.current_revision.public_id,
            install_script: "echo install".into(),
            verification_command: Some("test -f installed".into()),
            build_network: Some(SandboxNetworkPolicy::Allow),
            timeout_seconds: 600,
            metadata_path: None,
        },
        source_spec: template.current_revision.spec.clone(),
        status: SandboxBuildStatus::Queued,
        step: "queued".into(),
        logs: String::new(),
        error: None,
        software_notes: Some("# Installed tools\nAuthor supplied.".into()),
        snapshot_name: format!("test-{id}"),
        snapshot_id: None,
        snapshot_requested_at: None,
        builder_id: None,
        verification_sandbox_id: None,
        cancel_requested: false,
        cleanup_pending: false,
        applied_revision_id: None,
        created_at: now,
        updated_at: now,
    }
}
async fn ready(db: &StorageBackend, org: i64, build: &mut SandboxTemplateBuild) {
    build.status = SandboxBuildStatus::Ready;
    build.snapshot_id = Some(format!("snapshot-{}", build.id));
    db.save_sandbox_build(org, build).await.unwrap();
}

#[tokio::test]
async fn tenant_boundary_active_limit_and_cancellation_survive_runner_saves() {
    let (db, org, template) = setup().await;
    let mut first = attempt(&template);
    let second = attempt(&template);
    let (a, b) = tokio::join!(
        db.insert_sandbox_build(org, &first),
        db.insert_sandbox_build(org, &second)
    );
    assert_ne!(
        a.unwrap(),
        b.unwrap(),
        "one billed build per organization, even concurrently"
    );
    first = db
        .list_sandbox_builds(org, template.public_id, 0)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(
        db.get_sandbox_build(org + 1000, first.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.list_sandbox_builds(org + 1000, template.public_id, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!db.cancel_sandbox_build(org + 1000, first.id).await.unwrap());
    assert!(
        !db.discard_sandbox_build(org + 1000, first.id)
            .await
            .unwrap()
    );
    assert!(
        db.apply_sandbox_build(org + 1000, first.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.sandbox_build_credential(org, first.id).await.unwrap(),
        Some(vec![1, 2, 3])
    );
    assert!(
        db.sandbox_build_credential(org + 1000, first.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(db.cancel_sandbox_build(org, first.id).await.unwrap());
    first.logs = "Runner was holding an older copy".into();
    db.save_sandbox_build(org, &first).await.unwrap();
    assert!(
        db.get_sandbox_build(org, first.id)
            .await
            .unwrap()
            .unwrap()
            .cancel_requested
    );
    let account = template
        .current_revision
        .spec
        .target
        .credential
        .connection_id
        .unwrap();
    db.update_organization_connection(org, account, "Replacement account", &[4, 5, 6], None, None)
        .await
        .unwrap();
    assert_eq!(
        db.sandbox_build_credential(org, first.id).await.unwrap(),
        Some(vec![1, 2, 3]),
        "connection replacement cannot move accepted build authority"
    );
    first.status = SandboxBuildStatus::Cancelled;
    db.save_sandbox_build(org, &first).await.unwrap();
    assert!(
        db.sandbox_build_credential(org, first.id)
            .await
            .unwrap()
            .is_none(),
        "confirmed cleanup erases temporary authority"
    );
    assert!(db.discard_sandbox_build(org, first.id).await.unwrap());
    let discarded = db.get_sandbox_build(org, first.id).await.unwrap().unwrap();
    assert_eq!(discarded.status, SandboxBuildStatus::Discarded);
    assert!(
        !discarded.cleanup_pending,
        "discarding an already cleaned failed/cancelled attempt needs no provider authority"
    );
}

#[tokio::test]
async fn apply_is_atomic_idempotent_and_preserves_runtime_configuration() {
    let (db, org, template) = setup().await;
    let mut build = attempt(&template);
    db.insert_sandbox_build(org, &build).await.unwrap();
    assert!(db.apply_sandbox_build(org, build.id).await.is_err());
    ready(&db, org, &mut build).await;
    build.cleanup_pending = true;
    db.save_sandbox_build(org, &build).await.unwrap();
    assert!(db.apply_sandbox_build(org, build.id).await.is_err());
    build.cleanup_pending = false;
    db.save_sandbox_build(org, &build).await.unwrap();
    let (a, b) = tokio::join!(
        db.apply_sandbox_build(org, build.id),
        db.apply_sandbox_build(org, build.id)
    );
    let a = a.unwrap().unwrap();
    let b = b.unwrap().unwrap();
    assert_eq!(a.current_revision.public_id, b.current_revision.public_id);
    assert_eq!(a.current_revision.revision, 2);
    assert!(
        db.sandbox_build_credential(org, build.id)
            .await
            .unwrap()
            .is_none(),
        "applied audit records do not retain provider authority"
    );
    let mut expected = template.current_revision.spec.clone();
    expected
        .target
        .options
        .as_object_mut()
        .unwrap()
        .remove("size");
    expected.target.options["snapshot"] = json!(build.snapshot_id);
    assert_eq!(
        serde_json::to_value(&a.current_revision.spec).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let original = db
        .get_sandbox_template_revision(org, template.current_revision.public_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.spec.bootstrap.commands, ["echo runtime-bootstrap"]);
    assert!(
        original.spec.target.options.get("snapshot").is_none(),
        "existing revision pins remain unchanged"
    );
    assert!(!db.discard_sandbox_build(org, build.id).await.unwrap());
    // A stale runner cannot revive or delete an applied build.
    db.save_sandbox_build(org, &build).await.unwrap();
    assert_eq!(
        db.get_sandbox_build(org, build.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        SandboxBuildStatus::Applied
    );
    let provenance = db
        .sandbox_build_provenance(org, &a.current_revision.spec.target)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(provenance.build_id, build.id);
    assert_eq!(provenance.software_notes, build.software_notes);
    assert!(
        db.sandbox_build_provenance(org + 1000, &a.current_revision.spec.target)
            .await
            .unwrap()
            .is_none()
    );
    let mut wrong_account = a.current_revision.spec.target.clone();
    wrong_account.credential.connection_id = Some(uuid::Uuid::new_v4());
    assert!(
        db.sandbox_build_provenance(org, &wrong_account)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn stale_archived_and_managed_templates_cannot_receive_artifacts() {
    let (db, org, template) = setup().await;
    let mut build = attempt(&template);
    db.insert_sandbox_build(org, &build).await.unwrap();
    ready(&db, org, &mut build).await;
    db.revise_sandbox_template(
        org,
        template.public_id,
        &template.current_revision.spec,
        Some("Edited while building"),
        None,
    )
    .await
    .unwrap();
    assert!(
        db.apply_sandbox_build(org, build.id)
            .await
            .unwrap_err()
            .to_string()
            .contains("source template changed")
    );
    assert!(db.discard_sandbox_build(org, build.id).await.unwrap());
    db.save_sandbox_build(org, &build).await.unwrap();
    assert_eq!(
        db.get_sandbox_build(org, build.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        SandboxBuildStatus::Discarded
    );
    assert!(db.apply_sandbox_build(org, build.id).await.is_err());
    let template = db
        .get_sandbox_template(org, template.public_id)
        .await
        .unwrap()
        .unwrap();
    let mut build = attempt(&template);
    db.insert_sandbox_build(org, &build).await.unwrap();
    ready(&db, org, &mut build).await;
    db.archive_sandbox_template(org, template.public_id)
        .await
        .unwrap();
    assert!(db.apply_sandbox_build(org, build.id).await.is_err());
    let managed = db
        .create_sandbox_template(
            org,
            "managed-builder",
            "Managed",
            None,
            &template.current_revision.spec,
            true,
        )
        .await
        .unwrap();
    let mut build = attempt(&managed);
    db.insert_sandbox_build(org, &build).await.unwrap();
    ready(&db, org, &mut build).await;
    assert!(db.apply_sandbox_build(org, build.id).await.is_err());
}

#[tokio::test]
async fn api_artifact_actions_do_not_wait_for_a_long_running_builder() {
    let (db, org, template) = setup().await;
    let mut build = attempt(&template);
    db.insert_sandbox_build(org, &build).await.unwrap();
    ready(&db, org, &mut build).await;
    let mut owner = db.database().pool().begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,5479))")
        .bind(format!("sandbox-build-{}", build.id.uuid()))
        .execute(&mut *owner)
        .await
        .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        db.apply_sandbox_build(org, build.id),
    )
    .await
    .unwrap();
    assert!(result.unwrap_err().to_string().contains("retry shortly"));
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        db.discard_sandbox_build(org, build.id),
    )
    .await
    .unwrap();
    assert!(result.unwrap_err().to_string().contains("retry shortly"));
    owner.commit().await.unwrap();
    assert!(
        db.apply_sandbox_build(org, build.id)
            .await
            .unwrap()
            .is_some()
    );
}
