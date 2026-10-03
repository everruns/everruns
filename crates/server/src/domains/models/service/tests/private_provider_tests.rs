use super::*;
#[tokio::test]
async fn chatgpt_catalog_is_private_and_cannot_be_an_org_default() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let user = Uuid::new_v4();
    let mut owner = Caller::internal(DEFAULT_ORG_ID);
    owner.user_id = Some(user);
    let provider = db
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: "Personal".into(),
                provider_type: "chatgpt".into(),
                base_url: None,
                api_key_encrypted: None,
                settings: Some(serde_json::json!({"chatgpt":{"owner_user_id":user.to_string()}})),
            },
        )
        .await
        .unwrap();
    let model = service
        .create(&owner, provider.id.uuid(), build_create_request())
        .await
        .unwrap();
    let other = Caller::internal(DEFAULT_ORG_ID);
    assert_eq!(service.list_all(&owner).await.unwrap().len(), 1);
    assert!(service.list_all(&other).await.unwrap().is_empty());
    assert!(
        service
            .list_all_with_filters(&other, None, true, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        service
            .get_with_provider(&other, model.id.uuid())
            .await
            .is_err()
    );
    assert!(
        service
            .list_for_provider(&other, provider.id.uuid())
            .await
            .is_err()
    );
    assert!(service.delete(&other, model.id.uuid()).await.is_err());
    assert!(
        service
            .create(&other, provider.id.uuid(), build_create_request())
            .await
            .is_err()
    );
    assert!(
        service
            .set_default(DEFAULT_ORG_ID, model.id.uuid())
            .await
            .is_err()
    );
    assert!(
        !service
            .bootstrap_intelligence(DEFAULT_ORG_ID, provider.id.uuid())
            .await
            .unwrap()
            .changed()
    );
    assert!(
        db.get_default_model(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_none()
    );
}
