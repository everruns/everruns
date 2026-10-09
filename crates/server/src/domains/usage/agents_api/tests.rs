// Late Agents API usage (EVE-1145): a turn billed before the provider reported
// its usage is recorded as pending, and the reconciler applies the usage once,
// to the record, the session totals, and the budget ledger.

use super::*;
use crate::domains::budgets::service::LATE_USAGE_SOURCE;
use crate::domains::usage::UsageTrackingListener;
use crate::storage::models::*;
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::EventListener;
use everruns_core::events::{EventContext, LlmCostComponent, LlmGenerationData};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL: &str = "gpt-6-astra";
const TURN_PATH: &str = "/agents/sessions/sess_1/turns/turn_1";

struct Fixture {
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    session: SessionRow,
    provider_id: String,
    budget: BudgetRow,
}

async fn fixture(base_url: Option<&str>) -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    );
    let provider = db
        .create_provider(
            1,
            CreateProviderRow {
                name: "OpenAI".into(),
                provider_type: "openai".into(),
                base_url: base_url.map(str::to_string),
                api_key_encrypted: Some(encryption.encrypt_string("sk-test").unwrap()),
                settings: None,
            },
        )
        .await
        .unwrap();
    let session = db
        .create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("Late usage session".into()),
            capabilities: json!({}),
            mcp_servers: json!([]),
            initial_files: json!({}),
            ..Default::default()
        })
        .await
        .unwrap();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: session.id.to_string(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    Fixture {
        db,
        encryption,
        session,
        provider_id: provider.id.to_string(),
        budget,
    }
}

/// The `llm.generation` the driver emits when the turn's usage was still null
/// after its re-reads: tokens unknown, one priced web search.
fn unknown_usage_event(session_id: SessionId, provider_id: &str) -> Event {
    let mut data = LlmGenerationData::success(
        vec![],
        vec![],
        Some("done".into()),
        vec![],
        MODEL.into(),
        Some("openai".into()),
        None,
        None,
        None,
    )
    .with_cost_components(vec![
        LlmCostComponent::new(LlmCostComponent::MODEL_TOKENS, MODEL, None, None),
        LlmCostComponent::new(
            LlmCostComponent::HOSTED_TOOL,
            "web_search_call",
            Some(1),
            Some(0.01),
        ),
    ]);
    data.metadata.response_id = Some("turn_1".into());
    let mut event = Event::new(session_id, EventContext::empty(), data);
    event.metadata = Some(json!({
        "runtime_backend": "openai_agents_api",
        "provider_session_id": "sess_1",
        "provider_turn_id": "turn_1",
        "everruns_provider_id": provider_id,
    }));
    event
}

fn generation_data(event: &Event) -> &LlmGenerationData {
    match &event.data {
        everruns_core::events::EventData::LlmGeneration(data) => data,
        other => panic!("not a generation: {other:?}"),
    }
}

/// Bill the event the way the server does: usage tracking, then budgets.
async fn bill(f: &Fixture, event: &Event) {
    UsageTrackingListener::new(f.db.clone())
        .on_event(event)
        .await;
    BudgetService::new(f.db.clone()).on_event(event).await;
}

fn turn(usage: Value) -> Value {
    json!({"id": "turn_1", "status": "completed", "usage": usage})
}

async fn provider_reports(server: &MockServer, body: Value) {
    Mock::given(method("GET"))
        .and(path(TURN_PATH))
        .and(header("authorization", "Bearer sk-test"))
        .and(header("OpenAI-Beta", "agents=v1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn reconciler(f: &Fixture, server: &MockServer) -> AgentsApiUsageReconciler {
    AgentsApiUsageReconciler::new(f.db.clone(), Some(f.encryption.clone()))
        .with_base_url(server.uri())
}

async fn ledger(f: &Fixture) -> Vec<UsageLedgerRow> {
    f.db.list_usage_ledger_for_budget(f.budget.id, 10, 0)
        .await
        .unwrap()
}

async fn pending_id(f: &Fixture) -> Uuid {
    let rows =
        f.db.list_pending_usage_generations(i32::MAX, 10)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows[0].id
}

#[tokio::test]
async fn late_usage_lands_once_on_the_record_totals_and_budget() {
    let server = MockServer::start().await;
    let f = fixture(None).await;
    bill(&f, &unknown_usage_event(f.session.id, &f.provider_id)).await;

    // At billing time only the priced web search is known and debited.
    let id = pending_id(&f).await;
    let before = ledger(&f).await;
    assert_eq!(before.len(), 1);
    assert!((before[0].amount - 0.01).abs() < 1e-9);
    let session =
        f.db.get_session_unscoped(f.session.id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(session.total_input_tokens, 0);
    assert!((session.total_cost_usd - 0.01).abs() < 1e-9);

    // The provider has since filled the usage: 1000 input (200 cached), 100 out.
    provider_reports(
        &server,
        turn(json!({
            "input_tokens": 1000,
            "output_tokens": 100,
            "input_tokens_details": {"cached_tokens": 200}
        })),
    )
    .await;
    let tokens_cost = estimate_cost_usd(&DriverId::OpenAI, MODEL, 800, 100, 200, 0).unwrap();
    assert!(tokens_cost > 0.0);

    let svc = reconciler(&f, &server);
    assert_eq!(svc.reconcile().await.unwrap(), 1);

    let record = f.db.get_generation_usage(id).await.unwrap().unwrap();
    assert_eq!(
        (
            record.input_tokens,
            record.output_tokens,
            record.cache_read_tokens
        ),
        (800, 100, 200),
        "real token counts"
    );
    assert!(!record.usage_pending);
    let cost = record.estimated_cost_usd.unwrap();
    assert!((cost - (0.01 + tokens_cost)).abs() < 1e-9, "{cost}");

    let session =
        f.db.get_session_unscoped(f.session.id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(session.total_input_tokens, 800);
    assert_eq!(session.total_output_tokens, 100);
    assert_eq!(session.total_cache_read_tokens, 200);
    assert!((session.total_cost_usd - (0.01 + tokens_cost)).abs() < 1e-9);

    let after = ledger(&f).await;
    assert_eq!(after.len(), 2, "the late tokens are debited once");
    let late = after
        .iter()
        .find(|entry| (entry.amount - 0.01).abs() > 1e-9)
        .unwrap();
    assert!((late.amount - tokens_cost).abs() < 1e-9);
    let journal =
        f.db.get_usage_journal(late.journal_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(journal.source_type.as_deref(), Some(LATE_USAGE_SOURCE));
    assert_eq!(journal.source_id, Some(id.to_string()));
    assert_eq!(journal.measures["input_tokens"], 800);
    let budget = f.db.get_budget(1, f.budget.id).await.unwrap().unwrap();
    assert!((budget.balance - (10.0 - 0.01 - tokens_cost)).abs() < 1e-9);

    // Running the reconciler again debits nothing and changes no total.
    assert_eq!(svc.reconcile().await.unwrap(), 0);
    assert_eq!(svc.reconcile().await.unwrap(), 0);
    assert_eq!(ledger(&f).await.len(), 2, "no generation is debited twice");
    let again =
        f.db.get_session_unscoped(f.session.id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(again.total_input_tokens, 800);
    assert!((again.total_cost_usd - session.total_cost_usd).abs() < 1e-12);
    assert_eq!(
        f.db.get_generation_usage(id).await.unwrap().unwrap(),
        record
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn applying_late_usage_twice_applies_it_once() {
    let f = fixture(None).await;
    bill(&f, &unknown_usage_event(f.session.id, &f.provider_id)).await;
    let id = pending_id(&f).await;
    let usage = LateGenerationUsage {
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        estimated_cost_usd: Some(0.5),
    };
    assert!(f.db.apply_late_generation_usage(id, &usage).await.unwrap());
    assert!(
        !f.db.apply_late_generation_usage(id, &usage).await.unwrap(),
        "the second caller loses the usage_pending claim"
    );
    let record = f.db.get_generation_usage(id).await.unwrap().unwrap();
    assert!((record.estimated_cost_usd.unwrap() - 0.51).abs() < 1e-9);
}

#[tokio::test]
async fn usage_still_null_stays_unknown_and_debits_nothing() {
    let server = MockServer::start().await;
    let f = fixture(None).await;
    bill(&f, &unknown_usage_event(f.session.id, &f.provider_id)).await;
    let id = pending_id(&f).await;
    provider_reports(&server, turn(Value::Null)).await;

    assert_eq!(reconciler(&f, &server).reconcile().await.unwrap(), 0);

    let record = f.db.get_generation_usage(id).await.unwrap().unwrap();
    assert!(record.usage_pending);
    assert_eq!(record.input_tokens, 0);
    assert_eq!(record.reconciliation_attempts, 1);
    assert_eq!(ledger(&f).await.len(), 1, "only the priced tool");
    // Delayed: the next pass does not read it again right away.
    assert!(
        f.db.list_pending_usage_generations(MAX_ATTEMPTS, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_failed_lookup_is_retried_later_and_gives_up_after_the_last_attempt() {
    let server = MockServer::start().await;
    let f = fixture(None).await;
    bill(&f, &unknown_usage_event(f.session.id, &f.provider_id)).await;
    let id = pending_id(&f).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    assert_eq!(reconciler(&f, &server).reconcile().await.unwrap(), 0);
    assert_eq!(
        f.db.get_generation_usage(id)
            .await
            .unwrap()
            .unwrap()
            .reconciliation_attempts,
        1
    );
    assert_eq!(ledger(&f).await.len(), 1);
    // Every attempt spent: the record stays an explicit unknown.
    for _ in 1..MAX_ATTEMPTS {
        f.db.mark_llm_generation_reconciliation_failed(id, 0)
            .await
            .unwrap();
    }
    assert!(
        f.db.list_pending_usage_generations(MAX_ATTEMPTS, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn only_the_official_api_with_the_turns_own_provider_is_asked() {
    // TM-LLM-043: a provider moved off the official API is not sent the ids.
    let f = fixture(Some("https://gateway.example.com/v1")).await;
    let svc = AgentsApiUsageReconciler::new(f.db.clone(), Some(f.encryption.clone()));
    assert!(svc.client(1, &f.provider_id).await.is_none());

    let official = fixture(Some("https://api.openai.com/v1")).await;
    let svc = AgentsApiUsageReconciler::new(official.db.clone(), Some(official.encryption.clone()));
    assert!(svc.client(1, &official.provider_id).await.is_some());
    // Another org's provider, or no provider, resolves nothing.
    assert!(svc.client(2, &official.provider_id).await.is_none());
    assert!(svc.client(1, "not-a-provider-id").await.is_none());
    // Without the key's decryption service there are no credentials.
    let keyless = AgentsApiUsageReconciler::new(official.db.clone(), None);
    assert!(keyless.client(1, &official.provider_id).await.is_none());
}

#[tokio::test]
async fn a_missing_provider_delays_the_generation() {
    let server = MockServer::start().await;
    let f = fixture(None).await;
    bill(
        &f,
        &unknown_usage_event(f.session.id, "provider_0193b5a0000070008000000000000099"),
    )
    .await;
    let id = pending_id(&f).await;

    assert_eq!(reconciler(&f, &server).reconcile().await.unwrap(), 0);
    assert_eq!(
        f.db.get_generation_usage(id)
            .await
            .unwrap()
            .unwrap()
            .reconciliation_attempts,
        1
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn only_an_agents_api_generation_with_unknown_tokens_is_pending() {
    let provider = "provider_x";
    let event = unknown_usage_event(SessionId::new(), provider);
    let pending = pending_generation(&event, generation_data(&event), 7).unwrap();
    assert_eq!(pending.org_id, 7);
    assert_eq!(pending.provider_response_id, "turn_1");
    assert_eq!(pending.provider_session_id, "sess_1");
    assert_eq!(pending.provider_config_id, provider);
    assert_eq!(pending.estimated_cost_usd, Some(0.01));

    // A native generation is never pending.
    let mut native = event.clone();
    native.metadata = None;
    assert!(pending_generation(&native, generation_data(&native), 7).is_none());

    // Known usage is billed as usual.
    let mut known = event.clone();
    if let everruns_core::events::EventData::LlmGeneration(data) = &mut known.data {
        data.metadata.usage = Some(everruns_core::events::TokenUsage::new(1, 1));
    }
    assert!(pending_generation(&known, generation_data(&known), 7).is_none());

    // Without what the turn can be read back by, nothing is pending.
    for key in ["provider_session_id", "everruns_provider_id"] {
        let mut missing = event.clone();
        missing
            .metadata
            .as_mut()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(
            pending_generation(&missing, generation_data(&missing), 7).is_none(),
            "{key}"
        );
    }
    let mut no_turn = event.clone();
    if let everruns_core::events::EventData::LlmGeneration(data) = &mut no_turn.data {
        data.metadata.response_id = None;
    }
    assert!(pending_generation(&no_turn, generation_data(&no_turn), 7).is_none());
}
