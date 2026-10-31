use super::*;
use crate::domains::usage::tracking::UsageTrackingListener;
use crate::live_updates::event_delivery::EventDelivery;
use crate::services::EventService;
use crate::storage::encryption::{EncryptionService, generate_encryption_key};
use crate::storage::test_database::test_session_row;
use crate::storage::{CreateModelRow, CreateProviderRow};
use async_trait::async_trait;
use everruns_contracts::credential_schema::CredentialFormSchema;
use everruns_contracts::driver_registry::{
    DriverDescriptor, DriverId, DriverRegistry, EmbedRequest, EmbedResponse, EmbeddingsDriver,
    EmbeddingsDriverError, ServiceKind,
};
use sqlx::Row;

struct BilledEmbeddings;

#[async_trait]
impl EmbeddingsDriver for BilledEmbeddings {
    async fn embed(
        &self,
        _: &everruns_contracts::runtime_provider::ProviderEndpoint,
        request: EmbedRequest,
    ) -> Result<EmbedResponse, EmbeddingsDriverError> {
        Ok(EmbedResponse {
            embeddings: request.texts.iter().map(|_| vec![1.0, 0.0]).collect(),
            usage_tokens: Some(17),
            actual_cost_usd: Some(0.0002),
            response_id: Some("embedding-fixture-response".into()),
        })
    }
}

async fn assert_embedding_attribution(managed: bool, query: bool) {
    let mut service = tests::fixture();
    let mut collection =
        tests::create(&service, everruns_contracts::knowledge::IndexMode::Off).await;
    let encryption = EncryptionService::new(&generate_encryption_key("fixture"), &[]).unwrap();
    let provider = service
        .db
        .create_provider(
            1,
            CreateProviderRow {
                name: "Embedding billing fixture".into(),
                provider_type: "billing-embedding-fixture".into(),
                base_url: None,
                api_key_encrypted: Some(encryption.encrypt_string("fixture-key").unwrap()),
                settings: None,
            },
        )
        .await
        .unwrap();
    service
        .db
        .set_provider_managed(1, provider.id.uuid(), managed)
        .await
        .unwrap();
    let model = service
        .db
        .create_model(
            1,
            CreateModelRow {
                provider_id: provider.id,
                model_id: "fixture-embedding".into(),
                display_name: "Embedding billing fixture".into(),
                capabilities: vec!["embeddings".into()],
                enabled: true,
                is_favorite: false,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    collection.embedding_model_id = Some(model.id.uuid());
    let mut registry = DriverRegistry::new();
    registry.register_descriptor(DriverDescriptor {
        id: DriverId::external("billing-embedding-fixture"),
        display_name: "Embedding billing fixture".into(),
        services: vec![ServiceKind::Embeddings],
        credential_schema: CredentialFormSchema::empty(),
        base_url_env: None,
        oauth: None,
        recommended_models: None,
        chat: None,
        provider: None,
        embeddings: Some(Arc::new(|_| Box::new(BilledEmbeddings))),
    });
    service.drivers = Arc::new(registry);
    service.encryption = Some(Arc::new(encryption));
    if query {
        let session = service
            .db
            .create_session(test_session_row(1))
            .await
            .unwrap();
        service.query_session = Some(session.id);
        service.events = Some(Arc::new(EventService::with_listeners(
            service.db.clone(),
            EventDelivery::in_memory(),
            vec![Arc::new(UsageTrackingListener::new(service.db.clone()))],
        )));
    }
    let vectors = service
        .embed(&collection, vec!["query or document passage".into()])
        .await
        .unwrap();
    assert_eq!(vectors, vec![vec![1.0, 0.0]]);
    let rows = sqlx::query("SELECT provider_config_id, provider_response_id, managed, input_tokens, actual_cost_usd, session_id FROM llm_generations WHERE org_id=1")
        .fetch_all(service.db.pool()).await.unwrap();
    assert_eq!(rows.len(), 1, "each paid embedding call is metered once");
    assert_eq!(
        rows[0].get::<Option<String>, _>("provider_response_id"),
        Some("embedding-fixture-response".into())
    );
    assert_eq!(
        rows[0].get::<Option<String>, _>("provider_config_id"),
        Some(provider.id.to_string())
    );
    assert_eq!(
        rows[0].get::<bool, _>("managed"),
        managed,
        "BYOK and managed spend must stay distinct"
    );
    assert_eq!(rows[0].get::<i64, _>("input_tokens"), 17);
    assert_eq!(
        rows[0].get::<Option<f64>, _>("actual_cost_usd"),
        Some(0.0002)
    );
    assert_eq!(
        rows[0].get::<Option<Uuid>, _>("session_id").is_some(),
        query
    );
}

#[tokio::test]
async fn background_embeddings_retain_managed_and_byok_provider_attribution() {
    assert_embedding_attribution(true, false).await;
    assert_embedding_attribution(false, false).await;
}

#[tokio::test]
async fn query_embeddings_retain_managed_and_byok_provider_attribution() {
    assert_embedding_attribution(true, true).await;
    assert_embedding_attribution(false, true).await;
}

#[tokio::test]
async fn index_receipts_are_durable_idempotent_and_survive_collection_removal() {
    use everruns_turbopuffer::{Usage, UsageRecorder};
    let service = tests::fixture();
    let collection = tests::create(&service, everruns_contracts::knowledge::IndexMode::Off).await;
    let recorder = super::super::usage::IndexUsageRecorder {
        db: service.db.clone(),
        org_id: 1,
        collection_id: collection.id,
    };
    for _ in 0..2 {
        recorder
            .record(
                "immutable-batch",
                Usage {
                    written_bytes: 12345,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM knowledge_collections WHERE id=$1")
        .bind(collection.id)
        .execute(service.db.pool())
        .await
        .unwrap();
    let rows = sqlx::query(
        "SELECT written_bytes FROM knowledge_index_usage WHERE source_ref='immutable-batch'",
    )
    .fetch_all(service.db.pool())
    .await
    .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "a replay cannot create a second supplier receipt"
    );
    assert_eq!(rows[0].get::<i64, _>("written_bytes"), 12345);
}

#[tokio::test]
async fn index_receipts_do_not_wait_for_a_saturated_request_pool() {
    use crate::storage::repositories::{Database, DatabasePoolConfig};
    use everruns_turbopuffer::{Usage, UsageRecorder};
    use sqlx::ConnectOptions;
    use std::time::Duration;

    let service = tests::fixture();
    let collection = tests::create(&service, everruns_contracts::knowledge::IndexMode::Off).await;
    let url = service
        .db
        .pool()
        .connect_options()
        .to_url_lossy()
        .to_string();
    let database = Database::connect_with_config(
        &url,
        DatabasePoolConfig {
            max_connections: 1,
            min_connections: 1,
            background_max_connections: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let db = Arc::new(crate::storage::StorageBackend::from_database(database));
    let _request = db.pool().acquire().await.unwrap();
    let recorder = super::super::usage::IndexUsageRecorder {
        db: db.clone(),
        org_id: 1,
        collection_id: collection.id,
    };
    tokio::time::timeout(
        Duration::from_secs(2),
        recorder.record(
            "reserved-pool",
            Usage {
                written_bytes: 123,
                ..Default::default()
            },
        ),
    )
    .await
    .expect("receipt persistence must not need another request connection")
    .unwrap();
    let bytes: i64 = sqlx::query_scalar(
        "SELECT written_bytes FROM knowledge_index_usage WHERE source_ref='reserved-pool'",
    )
    .fetch_one(service.db.pool())
    .await
    .unwrap();
    assert_eq!(bytes, 123);
}
