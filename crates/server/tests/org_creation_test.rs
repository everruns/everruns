//! Organization creation tests
//!
//! Verifies org creation, membership assignment, validation constraints,
//! and listing of user organizations.
//!
//! Uses in-memory backend (no PostgreSQL required).
//!
//! Run with: cargo test -p everruns-server --test org_creation_test

use uuid::Uuid;

use everruns_server::storage::{CreateOrganizationRow, InMemoryDatabase};

// ============================================
// Creation Tests
// ============================================

#[tokio::test]
async fn test_create_organization_assigns_public_id() {
    let db = InMemoryDatabase::default();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_aaaabbbbccccddddeeee111122223333".to_string(),
            name: "Test Org".to_string(),
            created_by: None,
        })
        .await
        .expect("create should succeed");

    assert_eq!(org.public_id, "org_aaaabbbbccccddddeeee111122223333");
    assert_eq!(org.name, "Test Org");
    assert!(org.org_id > 1); // org_id=1 is default org
}

#[tokio::test]
async fn test_create_organization_multiple() {
    let db = InMemoryDatabase::default();

    let org1 = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Org A".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    let org2 = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Org B".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    assert_ne!(org1.org_id, org2.org_id);
    assert_ne!(org1.public_id, org2.public_id);
    assert_eq!(org1.name, "Org A");
    assert_eq!(org2.name, "Org B");
}

// ============================================
// Membership Tests
// ============================================

#[tokio::test]
async fn test_add_member_and_check() {
    let db = InMemoryDatabase::default();
    let user_id = Uuid::now_v7();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Members Test".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    // Initially not a member
    assert!(
        !db.is_organization_member(org.org_id, user_id)
            .await
            .unwrap()
    );

    // Add member
    let member = db
        .add_organization_member(org.org_id, user_id, "member")
        .await
        .unwrap();
    assert_eq!(member.org_id, org.org_id);
    assert_eq!(member.user_id, user_id);

    // Now a member
    assert!(
        db.is_organization_member(org.org_id, user_id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_list_user_organizations() {
    let db = InMemoryDatabase::default();
    let user_id = Uuid::now_v7();

    // Create two orgs and add user to both
    let org1 = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "User Org 1".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    let org2 = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "User Org 2".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    db.add_organization_member(org1.org_id, user_id, "member")
        .await
        .unwrap();
    db.add_organization_member(org2.org_id, user_id, "member")
        .await
        .unwrap();

    let user_orgs = db.list_user_organizations(user_id).await.unwrap();
    assert_eq!(user_orgs.len(), 2);

    let names: Vec<&str> = user_orgs.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"User Org 1"));
    assert!(names.contains(&"User Org 2"));
}

#[tokio::test]
async fn test_list_user_organizations_excludes_non_member() {
    let db = InMemoryDatabase::default();
    let user_id = Uuid::now_v7();
    let other_user_id = Uuid::now_v7();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Exclusive Org".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    db.add_organization_member(org.org_id, user_id, "member")
        .await
        .unwrap();

    // user_id can see it
    let user_orgs = db.list_user_organizations(user_id).await.unwrap();
    assert_eq!(user_orgs.len(), 1);

    // other_user_id cannot
    let other_orgs = db.list_user_organizations(other_user_id).await.unwrap();
    assert!(other_orgs.is_empty());
}

#[tokio::test]
async fn test_remove_member() {
    let db = InMemoryDatabase::default();
    let user_id = Uuid::now_v7();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Remove Test".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    db.add_organization_member(org.org_id, user_id, "member")
        .await
        .unwrap();
    assert!(
        db.is_organization_member(org.org_id, user_id)
            .await
            .unwrap()
    );

    // Remove member
    let removed = db
        .remove_organization_member(org.org_id, user_id)
        .await
        .unwrap();
    assert!(removed);

    // No longer a member
    assert!(
        !db.is_organization_member(org.org_id, user_id)
            .await
            .unwrap()
    );
    assert!(
        db.list_user_organizations(user_id)
            .await
            .unwrap()
            .is_empty()
    );
}

// ============================================
// Lookup Tests
// ============================================

#[tokio::test]
async fn test_get_organization_by_public_id() {
    let db = InMemoryDatabase::default();
    let public_id = format!("org_{}", Uuid::now_v7().simple());

    db.create_organization(CreateOrganizationRow {
        public_id: public_id.clone(),
        name: "Lookup Test".to_string(),
        created_by: None,
    })
    .await
    .unwrap();

    let found = db.get_organization_by_public_id(&public_id).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Lookup Test");

    let not_found = db
        .get_organization_by_public_id("org_00000000000000000000000000009999")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_default_org_exists() {
    let db = InMemoryDatabase::default();

    // Default org (org_id=1) should exist
    let default_org = db.get_organization(1).await.unwrap();
    assert!(default_org.is_some());

    let org = default_org.unwrap();
    assert_eq!(org.org_id, 1);
    assert_eq!(org.public_id, "org_00000000000000000000000000000001");
    assert_eq!(org.name, "Default Organization");
}

// ============================================
// Update & Delete Tests
// ============================================

#[tokio::test]
async fn test_update_organization_name() {
    let db = InMemoryDatabase::default();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Original Name".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    let updated = db
        .update_organization(
            org.org_id,
            everruns_server::storage::UpdateOrganization {
                name: Some("New Name".to_string()),
            },
        )
        .await
        .unwrap();

    assert!(updated.is_some());
    assert_eq!(updated.unwrap().name, "New Name");
}

#[tokio::test]
async fn test_delete_organization() {
    let db = InMemoryDatabase::default();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Delete Me".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    let deleted = db.delete_organization(org.org_id).await.unwrap();
    assert!(deleted);

    let gone = db.get_organization(org.org_id).await.unwrap();
    assert!(gone.is_none());
}

// ============================================
// Validation Tests (core)
// ============================================

#[test]
fn test_validate_org_public_id() {
    use everruns_core::validate_org_public_id;

    assert!(validate_org_public_id(
        "org_00000000000000000000000000000001"
    ));
    assert!(validate_org_public_id(
        "org_aaaabbbbccccddddeeee111122223333"
    ));
    assert!(!validate_org_public_id("org_"));
    assert!(!validate_org_public_id("org_short"));
    assert!(!validate_org_public_id(
        "agent_00000000000000000000000000000001"
    ));
    assert!(!validate_org_public_id(""));
    assert!(!validate_org_public_id(
        "org_AAAABBBBCCCCDDDDEEEE111122223333"
    )); // uppercase
}

#[test]
fn test_generate_org_public_id_format() {
    use everruns_core::{generate_org_public_id, validate_org_public_id};

    for _ in 0..10 {
        let id = generate_org_public_id();
        assert!(
            validate_org_public_id(&id),
            "Generated ID should be valid: {id}"
        );
        assert!(id.starts_with("org_"));
        assert_eq!(id.len(), 36); // "org_" + 32 hex chars
    }
}

// ============================================
// Membership Sync / Reconciliation Tests
// ============================================

#[tokio::test]
async fn test_ensure_membership_updates_role() {
    let db = InMemoryDatabase::default();
    let user_id = Uuid::now_v7();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Role Update Test".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    // Insert as member
    db.ensure_membership(user_id, org.org_id, "member")
        .await
        .unwrap();
    let members = db.list_organization_members(org.org_id).await.unwrap();
    assert_eq!(members[0].role, "member");

    // Ensure again with different role — should update
    db.ensure_membership(user_id, org.org_id, "admin")
        .await
        .unwrap();
    let members = db.list_organization_members(org.org_id).await.unwrap();
    assert_eq!(members.len(), 1, "should still have one member");
    assert_eq!(members[0].role, "admin");
}

#[tokio::test]
async fn test_reconcile_memberships_add_update_remove() {
    let db = InMemoryDatabase::default();
    let user_a = Uuid::now_v7();
    let user_b = Uuid::now_v7();
    let user_c = Uuid::now_v7();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Reconcile Test".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    // Seed: A=owner, B=member
    db.add_organization_member(org.org_id, user_a, "owner")
        .await
        .unwrap();
    db.add_organization_member(org.org_id, user_b, "member")
        .await
        .unwrap();

    // Authoritative: A=admin (role change), C=member (new), B absent (remove)
    let authoritative = vec![
        (user_a, "admin".to_string()),
        (user_c, "member".to_string()),
    ];

    let (added, updated, removed) = db
        .reconcile_memberships(org.org_id, &authoritative)
        .await
        .unwrap();

    assert_eq!(added, 1, "user_c should be added");
    assert_eq!(updated, 1, "user_a role should be updated");
    assert_eq!(removed, 1, "user_b should be removed");

    // Verify final state
    let members = db.list_organization_members(org.org_id).await.unwrap();
    assert_eq!(members.len(), 2);

    let member_map: std::collections::HashMap<Uuid, String> =
        members.into_iter().map(|m| (m.user_id, m.role)).collect();
    assert_eq!(member_map.get(&user_a).unwrap(), "admin");
    assert_eq!(member_map.get(&user_c).unwrap(), "member");
    assert!(!member_map.contains_key(&user_b));
}

#[tokio::test]
async fn test_reconcile_memberships_empty_authoritative_removes_all() {
    let db = InMemoryDatabase::default();
    let user_id = Uuid::now_v7();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Reconcile Empty Test".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    db.add_organization_member(org.org_id, user_id, "owner")
        .await
        .unwrap();

    let (added, updated, removed) = db.reconcile_memberships(org.org_id, &[]).await.unwrap();

    assert_eq!(added, 0);
    assert_eq!(updated, 0);
    assert_eq!(removed, 1);

    let members = db.list_organization_members(org.org_id).await.unwrap();
    assert!(members.is_empty());
}

#[tokio::test]
async fn test_reconcile_memberships_no_changes() {
    let db = InMemoryDatabase::default();
    let user_id = Uuid::now_v7();

    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Reconcile Noop Test".to_string(),
            created_by: None,
        })
        .await
        .unwrap();

    db.add_organization_member(org.org_id, user_id, "owner")
        .await
        .unwrap();

    let authoritative = vec![(user_id, "owner".to_string())];
    let (added, updated, removed) = db
        .reconcile_memberships(org.org_id, &authoritative)
        .await
        .unwrap();

    assert_eq!(added, 0);
    assert_eq!(updated, 0);
    assert_eq!(removed, 0);
}
