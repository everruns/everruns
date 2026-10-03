use super::*;
use crate::storage::models::{CreateProviderRow, CreateSessionRow};
use everruns_contracts::typed_id::PrincipalId;

#[tokio::test]
async fn chatgpt_requires_the_exact_personal_session_owner() {
    let db = StorageBackend::in_memory();
    let owner = PrincipalId::new();
    let provider = db.create_provider(1, CreateProviderRow {
        name: "Personal".into(), provider_type:"chatgpt".into(), base_url:None, api_key_encrypted:None,
        settings:Some(json!({"chatgpt":{"owner_principal_id":owner.to_string(),"owner_user_id":uuid::Uuid::new_v4().to_string()}})),
    }).await.unwrap();
    let personal = db
        .create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: owner,
            ..Default::default()
        })
        .await
        .unwrap();
    let other = db
        .create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::new(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        check_session(&db, &provider, Some(personal.id.uuid()))
            .await
            .is_ok()
    );
    assert!(
        check_session(&db, &provider, Some(other.id.uuid()))
            .await
            .is_err()
    );
    assert!(check_session(&db, &provider, None).await.is_err());
    let mut corrupt = provider.clone();
    corrupt.settings = json!({});
    assert!(
        check_session(&db, &corrupt, Some(personal.id.uuid()))
            .await
            .is_err()
    );
}
#[test]
fn management_owner_is_independent_of_org_admin_permissions() {
    let user = uuid::Uuid::new_v4();
    let settings = json!({"chatgpt":{"owner_user_id":user.to_string()}});
    let mut owner = Caller::internal(1);
    owner.user_id = Some(user);
    assert!(visible(&settings, &owner));
    assert!(!visible(&settings, &Caller::internal(1)));
    owner.user_id = Some(uuid::Uuid::new_v4());
    assert!(!visible(&settings, &owner));
    assert!(visible(&json!({}), &owner));
}
