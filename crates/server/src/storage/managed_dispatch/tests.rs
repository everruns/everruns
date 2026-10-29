use super::*;
use crate::storage::{CreateProviderRow, StorageBackend};
use async_trait::async_trait;
use everruns_contracts::http_transport::{
    HttpError, HttpRequest, HttpResponse, HttpTransport, StatusCode,
};
use everruns_contracts::managed_dispatch::*;
use everruns_contracts::runtime_provider::{BearerAuth, ProviderEndpoint};
use everruns_contracts::typed_id::ProviderId;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
struct Host {
    identity: DispatchIdentity,
    receipt: Arc<Mutex<Option<SupplierReceipt>>>,
    journal: Option<StorageBackend>,
    fail_journal: bool,
}
#[async_trait]
impl DispatchHost for Host {
    type Permit = Self;
    async fn reserve(&self, _: &DispatchPlan) -> std::result::Result<Self, DispatchError> {
        Ok(self.clone())
    }
}
#[async_trait]
impl DispatchPermit for Host {
    async fn start(
        &mut self,
        _: &DispatchPlan,
    ) -> std::result::Result<(DispatchIdentity, ProviderEndpoint), DispatchError> {
        Ok((
            self.identity.clone(),
            ProviderEndpoint::from_parts(
                OPENROUTER_API,
                BearerAuth::new(Uuid::new_v4().to_string()),
            ),
        ))
    }
    async fn record(
        &mut self,
        receipt: &SupplierReceipt,
    ) -> std::result::Result<(), DispatchError> {
        if let Some(db) = &self.journal {
            let mut tx = everruns_durable::begin_transaction(db.pool())
                .await
                .map_err(|_| DispatchError::Uncertain)?;
            insert_host_generation(&mut tx, 1, receipt)
                .await
                .map_err(|_| DispatchError::Uncertain)?;
            if self.fail_journal {
                tx.rollback().await.map_err(|_| DispatchError::Uncertain)?;
                return Err(DispatchError::Uncertain);
            }
            everruns_durable::commit_transaction(tx)
                .await
                .map_err(|_| DispatchError::Uncertain)?;
        }
        *self.receipt.lock().unwrap() = Some(receipt.clone());
        Ok(())
    }
}
struct Transport;
#[async_trait]
impl HttpTransport for Transport {
    async fn execute(&self, _: HttpRequest) -> std::result::Result<HttpResponse, HttpError> {
        let body=serde_json::to_vec(&serde_json::json!({
            "id":"gen-journal-fixture","model":"openai/fixture","service_tier":"default",
            "openrouter_metadata":{"attempt":1,"requested":"openai/fixture","is_byok":false,
                "endpoints":{"available":[{"model":"openai/fixture","provider":"OpenAI","selected":true}]}},
            "usage":{"prompt_tokens":20,"completion_tokens":3,"cost":0.000123},
            "choices":[{"message":{"content":"done"}}]
        })).unwrap();
        Ok(HttpResponse::new(
            StatusCode::OK,
            Default::default(),
            Box::pin(futures::stream::once(async { Ok(body.into()) })),
        ))
    }
}
async fn fixture() -> (StorageBackend, Host) {
    let db = StorageBackend::test_database();
    let organization: String =
        sqlx::query_scalar("SELECT public_id FROM organizations WHERE org_id=1")
            .fetch_one(db.pool())
            .await
            .unwrap();
    let provider = db
        .create_provider(
            1,
            CreateProviderRow {
                name: "managed fixture".into(),
                provider_type: "openrouter".into(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    db.set_provider_managed(1, provider.id.uuid(), true)
        .await
        .unwrap();
    let host = Host {
        identity: DispatchIdentity {
            generation_id: Uuid::new_v4(),
            organization,
            session_id: None,
            supplier_account: "fixture-account".into(),
            provider_id: provider.id,
            started_at: chrono::Utc::now(),
        },
        receipt: Arc::default(),
        journal: None,
        fail_journal: false,
    };
    (db, host)
}
async fn observed(host: &Host) -> SupplierReceipt {
    let policy = TextDispatchPolicy {
        profile_id: "journal-fixture".into(),
        model: "openai/fixture".into(),
        context_window_tokens: 1024,
        max_output_tokens: 32,
        prompt_usd_per_million: 1,
        completion_usd_per_million: 2,
        max_request_bytes: 4096,
        max_response_bytes: 4096,
        timeout: std::time::Duration::from_secs(1),
    };
    let request = TextRequest {
        messages: vec![TextMessage {
            role: TextRole::User,
            content: "hello".into(),
        }],
        max_output_tokens: 32,
    };
    dispatch_text(
        host,
        &DispatchTransport::injected(Arc::new(Transport)),
        &policy,
        &request,
    )
    .await
    .unwrap();
    host.receipt.lock().unwrap().as_ref().unwrap().clone()
}

#[tokio::test]
async fn supplier_output_waits_for_the_real_host_journal_commit_and_never_bypasses_rollback() {
    for fail in [false, true] {
        let (db, mut host) = fixture().await;
        host.journal = Some(db.clone());
        host.fail_journal = fail;
        let policy = TextDispatchPolicy {
            profile_id: "transactional-host-fixture".into(),
            model: "openai/fixture".into(),
            context_window_tokens: 1024,
            max_output_tokens: 32,
            prompt_usd_per_million: 1,
            completion_usd_per_million: 2,
            max_request_bytes: 4096,
            max_response_bytes: 4096,
            timeout: std::time::Duration::from_secs(1),
        };
        let request = TextRequest {
            messages: vec![TextMessage {
                role: TextRole::User,
                content: "hello".into(),
            }],
            max_output_tokens: 32,
        };
        let result = dispatch_text(
            &host,
            &DispatchTransport::injected(Arc::new(Transport)),
            &policy,
            &request,
        )
        .await;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM llm_generations WHERE id=$1")
            .bind(host.identity.generation_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
        if fail {
            assert_eq!(result, Err(DispatchError::Uncertain));
            assert_eq!(count, 0);
        } else {
            assert_eq!(result, Ok("done".into()));
            assert_eq!(count, 1);
        }
    }
}
#[tokio::test]
async fn canonical_host_receipt_keeps_preallocated_identity_and_commits_or_rolls_back_with_owner() {
    let (db, host) = fixture().await;
    let receipt = observed(&host).await;
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(insert_host_generation(&mut tx, 1, &receipt).await.unwrap());
    tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM llm_generations WHERE id=$1")
        .bind(host.identity.generation_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(insert_host_generation(&mut tx, 1, &receipt).await.unwrap());
    assert!(!insert_host_generation(&mut tx, 1, &receipt).await.unwrap());
    everruns_durable::commit_transaction(tx).await.unwrap();
    let (id,input,output,provider,managed,event):(Uuid,i64,i64,String,bool,Option<Uuid>)=sqlx::query_as("SELECT id,input_tokens,output_tokens,provider_config_id,managed,event_id FROM llm_generations WHERE id=$1").bind(host.identity.generation_id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(
        (id, input, output, provider, managed, event),
        (
            host.identity.generation_id,
            20,
            3,
            host.identity.provider_id.to_string(),
            true,
            None
        )
    );
}
#[tokio::test]
async fn host_journal_refuses_foreign_org_provider_and_legacy_generation_collisions() {
    let (db, mut host) = fixture().await;
    let receipt = observed(&host).await;
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(
        insert_host_generation(&mut tx, 999, &receipt)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    sqlx::query("INSERT INTO llm_generations(id,org_id,model) VALUES($1,1,'legacy-observation')")
        .bind(host.identity.generation_id)
        .execute(db.pool())
        .await
        .unwrap();
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(insert_host_generation(&mut tx, 1, &receipt).await.is_err());
    tx.rollback().await.unwrap();
    let model: String = sqlx::query_scalar("SELECT model FROM llm_generations WHERE id=$1")
        .bind(host.identity.generation_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(model, "legacy-observation");
    let org = db
        .create_organization(crate::storage::CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::new_v4().simple()),
            name: "foreign".into(),
            created_by: None,
        })
        .await
        .unwrap();
    let foreign = db
        .create_provider(
            org.org_id,
            CreateProviderRow {
                name: "foreign".into(),
                provider_type: "openrouter".into(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    host.identity.generation_id = Uuid::new_v4();
    host.identity.provider_id = foreign.id;
    let receipt = observed(&host).await;
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(insert_host_generation(&mut tx, 1, &receipt).await.is_err());
    tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM llm_generations WHERE id=$1")
        .bind(host.identity.generation_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn deleting_the_original_provider_does_not_erase_completed_supplier_liability() {
    let (db, host) = fixture().await;
    let receipt = observed(&host).await;
    sqlx::query("DELETE FROM providers WHERE id=$1")
        .bind(host.identity.provider_id)
        .execute(db.pool())
        .await
        .unwrap();
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(insert_host_generation(&mut tx, 1, &receipt).await.unwrap());
    everruns_durable::commit_transaction(tx).await.unwrap();
    let original: ProviderId = host.identity.provider_id;
    let stored: String =
        sqlx::query_scalar("SELECT provider_config_id FROM llm_generations WHERE id=$1")
            .bind(host.identity.generation_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(stored, original.to_string());
}

#[tokio::test]
async fn a_foreign_session_refuses_but_a_deleted_original_session_keeps_org_liability() {
    use crate::storage::{CreateOrganizationRow, CreateSessionRow};
    use everruns_contracts::typed_id::PrincipalId;
    let (db, mut host) = fixture().await;
    let foreign = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::new_v4().simple()),
            name: "foreign session org".into(),
            created_by: None,
        })
        .await
        .unwrap();
    let foreign_session = db
        .create_session(CreateSessionRow {
            org_id: foreign.org_id,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("foreign".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    host.identity.session_id = Some(foreign_session.id.uuid());
    let receipt = observed(&host).await;
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(insert_host_generation(&mut tx, 1, &receipt).await.is_err());
    tx.rollback().await.unwrap();
    let own = db
        .create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("original".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    host.identity.session_id = Some(own.id.uuid());
    let receipt = observed(&host).await;
    sqlx::query("DELETE FROM sessions WHERE id=$1")
        .bind(own.id.uuid())
        .execute(db.pool())
        .await
        .unwrap();
    let mut tx = everruns_durable::begin_transaction(db.pool())
        .await
        .unwrap();
    assert!(insert_host_generation(&mut tx, 1, &receipt).await.unwrap());
    everruns_durable::commit_transaction(tx).await.unwrap();
    let stored: (i64, Option<Uuid>) =
        sqlx::query_as("SELECT org_id,session_id FROM llm_generations WHERE id=$1")
            .bind(host.identity.generation_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(stored, (1, None));
    assert_eq!(receipt.identity().session_id, Some(own.id.uuid()));
}
