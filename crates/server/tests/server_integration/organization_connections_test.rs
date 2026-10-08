//! PostgreSQL contracts for organization-owned sandbox provider accounts.

use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;
use everruns_server::records::{
    SandboxBootstrap, SandboxLifecycle, SandboxTargetSpec, SandboxTemplateSpec,
    generate_org_public_id,
};
use everruns_server::storage::{
    CreateOrganizationConnectionRow, CreateOrganizationRow, StorageBackend,
};

async fn organization(db: &StorageBackend, name: &str) -> i64 {
    db.create_organization(CreateOrganizationRow {
        public_id: generate_org_public_id(),
        name: name.to_string(),
        created_by: None,
    })
    .await
    .expect("create organization")
    .org_id
}

#[tokio::test]
async fn organization_connections_are_named_exact_and_tenant_scoped() {
    let db = StorageBackend::test_database();
    let first_org = organization(&db, "Sandbox account A").await;
    let second_org = organization(&db, "Sandbox account B").await;

    let first = db
        .create_organization_connection(CreateOrganizationConnectionRow {
            org_id: first_org,
            name: "Daytona production".to_string(),
            provider: "daytona".to_string(),
            access_token_encrypted: vec![1, 2, 3],
            provider_username: None,
            provider_metadata: None,
        })
        .await
        .expect("create first account");
    let second = db
        .create_organization_connection(CreateOrganizationConnectionRow {
            org_id: second_org,
            name: "Daytona other tenant".to_string(),
            provider: "daytona".to_string(),
            access_token_encrypted: vec![4, 5, 6],
            provider_username: None,
            provider_metadata: None,
        })
        .await
        .expect("create second account");

    let listed = db
        .list_organization_connections(first_org)
        .await
        .expect("list first tenant accounts");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, first.id);
    assert_eq!(listed[0].name.as_deref(), Some("Daytona production"));
    assert_ne!(first.virtual_user_id, second.virtual_user_id);
    assert!(
        db.get_organization_connection(first_org, second.id)
            .await
            .expect("tenant-scoped lookup")
            .is_none()
    );
    assert!(
        db.get_virtual_user_connection_by_id(first.virtual_user_id, first.id, "daytona",)
            .await
            .expect("exact account lookup")
            .is_some()
    );

    let mut target = SandboxTargetSpec::managed("daytona");
    target.credential.source = SessionSandboxCredentialSource::Organization;
    target.credential.connection_id = Some(first.id);
    let template = db
        .create_sandbox_template(
            first_org,
            "daytona-production",
            "Daytona production",
            None,
            &SandboxTemplateSpec {
                template_revision_id: None,
                target,
                containment: None,
                durability: None,
                lifecycle: SandboxLifecycle::default(),
                bootstrap: SandboxBootstrap::default(),
            },
            false,
        )
        .await
        .expect("create referencing Sandbox Template");
    assert!(
        db.delete_organization_connection(first_org, first.id)
            .await
            .is_err(),
        "a current Sandbox Template must keep its provider account"
    );
    db.revise_sandbox_template(
        first_org,
        template.public_id,
        &SandboxTemplateSpec {
            template_revision_id: None,
            target: SandboxTargetSpec::vfs("bashkit"),
            containment: None,
            durability: None,
            lifecycle: SandboxLifecycle::default(),
            bootstrap: SandboxBootstrap::default(),
        },
        None,
        None,
    )
    .await
    .expect("revise template")
    .expect("template still exists");

    assert!(
        db.delete_organization_connection(first_org, first.id)
            .await
            .expect("delete unreferenced account")
    );
    assert!(
        db.get_organization_connection(first_org, first.id)
            .await
            .expect("lookup deleted account")
            .is_none()
    );
}
