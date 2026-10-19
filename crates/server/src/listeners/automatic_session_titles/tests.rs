use super::*;
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{
    CreateModelRow, CreateProviderRow, CreateSessionRow, UpdateField, UpdateOrganizationSettings,
    UpdateSession,
};
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::{InputMessageData, RuntimeMessage, TurnCompletedData};
use std::collections::VecDeque;
use tokio::sync::{Mutex, Notify};

struct FakeGenerator {
    replies: Mutex<VecDeque<String>>,
    models: Mutex<Vec<ModelId>>,
    pause: Option<(Arc<Notify>, Arc<Notify>)>,
}
#[async_trait]
impl TitleGenerator for FakeGenerator {
    async fn generate(
        &self,
        _: i64,
        _: SessionId,
        model: ModelId,
        _: String,
    ) -> Result<GeneratedTitle> {
        self.models.lock().await.push(model);
        if let Some((started, release)) = &self.pause {
            started.notify_one();
            release.notified().await;
        }
        Ok(GeneratedTitle {
            text: self
                .replies
                .lock()
                .await
                .pop_front()
                .context("No fake reply")?,
            generation: None,
        })
    }
}
struct Fixture {
    db: Arc<StorageBackend>,
    events: Arc<EventService>,
    listener: AutomaticSessionTitlesListener,
    generator: Arc<FakeGenerator>,
    id: SessionId,
    model: ModelId,
}
fn enabled(config: serde_json::Value) -> serde_json::Value {
    json!([{"ref": CAPABILITY_ID, "config": config}])
}
async fn fixture(caps: serde_json::Value, replies: &[&str], pause: bool) -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    crate::setup::org_init::initialize_org_harnesses(&db, 1)
        .await
        .unwrap();
    let harness = crate::setup::org_init::base_harness_id(&db, 1)
        .await
        .unwrap();
    let provider = db
        .create_provider(
            1,
            CreateProviderRow {
                name: "Title test".into(),
                provider_type: "openai".into(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    let model = db
        .create_model(
            1,
            CreateModelRow {
                provider_id: provider.id,
                model_id: "title-test".into(),
                display_name: "Title test".into(),
                capabilities: vec![],
                is_favorite: false,
                enabled: true,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap()
        .id;
    let session = db
        .create_session(CreateSessionRow {
            org_id: 1,
            owner_principal_id: PrincipalId::from_seed(1),
            harness_id: Some(harness),
            model_id: Some(model),
            capabilities: caps,
            ..Default::default()
        })
        .await
        .unwrap();
    let events = Arc::new(EventService::new(db.clone(), EventDelivery::in_memory()));
    let generator = Arc::new(FakeGenerator {
        replies: Mutex::new(replies.iter().map(|s| (*s).to_string()).collect()),
        models: Mutex::new(vec![]),
        pause: pause.then(|| (Arc::new(Notify::new()), Arc::new(Notify::new()))),
    });
    let listener = AutomaticSessionTitlesListener::new(
        db.clone(),
        SessionService::new(db.clone()),
        generator.clone(),
    );
    listener.bind_events(&events);
    let f = Fixture {
        db,
        events,
        listener,
        generator,
        id: session.id,
        model,
    };
    f.inputs(1).await;
    f
}
impl Fixture {
    async fn inputs(&self, count: usize) {
        for _ in 0..count {
            self.events
                .emit(EventRequest::new(
                    self.id,
                    EventContext::empty(),
                    InputMessageData::new(RuntimeMessage::user("Plan a database migration")),
                ))
                .await
                .unwrap();
        }
    }
    async fn turn(&self) -> Result<()> {
        let event = self
            .events
            .emit(EventRequest::new(
                self.id,
                EventContext::empty(),
                TurnCompletedData::default(),
            ))
            .await?;
        self.listener
            .generate_for_turn(self.id, event.sequence.unwrap())
            .await
    }
    async fn age(&self) {
        sqlx::query("UPDATE sessions SET auto_title_checked_at = now() - interval '6 minutes' WHERE id = $1").bind(self.id).execute(self.db.pool()).await.unwrap();
    }
    async fn title(&self) -> Option<String> {
        self.db
            .get_session(1, self.id)
            .await
            .unwrap()
            .unwrap()
            .title
    }
    async fn calls(&self) -> usize {
        self.generator.models.lock().await.len()
    }
    async fn small(&self) -> ModelId {
        let row = self
            .db
            .get_model(1, self.model.uuid())
            .await
            .unwrap()
            .unwrap();
        let small = self
            .db
            .create_model(
                1,
                CreateModelRow {
                    provider_id: row.provider_id,
                    model_id: "small-test".into(),
                    display_name: "Small".into(),
                    capabilities: vec![],
                    is_favorite: false,
                    enabled: true,
                    source: "manual".into(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap()
            .id;
        self.db
            .patch_organization_settings(
                1,
                UpdateOrganizationSettings {
                    small_model_id: UpdateField::Set(small),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        small
    }
}

#[tokio::test]
async fn first_title_then_sparse_keep_and_retitle() {
    let f = fixture(
        enabled(json!({})),
        &[
            r#"{"title":"Plan database migration"}"#,
            r#"{"title":null}"#,
            r#"{"title":"Plan summer holiday"}"#,
        ],
        false,
    )
    .await;
    f.turn().await.unwrap();
    assert_eq!(f.title().await.as_deref(), Some("Plan database migration"));
    for _ in 0..100 {
        f.turn().await.unwrap();
    }
    assert_eq!(f.calls().await, 1);
    f.inputs(4).await;
    f.turn().await.unwrap(); // cooldown still holds
    assert_eq!(f.calls().await, 1);
    f.age().await;
    f.turn().await.unwrap();
    f.inputs(7).await;
    f.age().await;
    f.turn().await.unwrap();
    assert_eq!(f.calls().await, 2);
    f.inputs(1).await;
    f.turn().await.unwrap();
    assert_eq!(f.title().await.as_deref(), Some("Plan summer holiday"));
    let events =
        f.db.list_events(
            f.id,
            None,
            None,
            &["session.title.updated".into()],
            &[],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].data["previous_title"], "Plan database migration");
}

#[tokio::test]
async fn stable_topics_back_off_and_new_titles_reset_interval() {
    let f = fixture(
        enabled(json!({})),
        &[
            r#"{"title":"Plan database migration"}"#,
            r#"{"title":null}"#,
            r#"{"title":null}"#,
            r#"{"title":null}"#,
            r#"{"title":null}"#,
            r#"{"title":"Plan database migration"}"#,
        ],
        false,
    )
    .await;
    f.turn().await.unwrap();
    for _ in 0..100 {
        f.inputs(1).await;
        f.age().await;
        f.turn().await.unwrap();
    }
    assert_eq!(f.calls().await, 6); // 1 + checks at 4, 12, 28, 60, 92
    let g = fixture(
        enabled(json!({})),
        &[
            r#"{"title":"First topic title"}"#,
            r#"{"title":null}"#,
            r#"{"title":"Second topic title"}"#,
            r#"{"title":null}"#,
        ],
        false,
    )
    .await;
    g.turn().await.unwrap();
    for count in [4, 8, 4] {
        g.inputs(count).await;
        g.age().await;
        g.turn().await.unwrap();
    }
    assert_eq!(g.calls().await, 4);
}

#[tokio::test]
async fn opt_in_and_initial_only_policy() {
    let absent = fixture(json!([]), &[], false).await;
    absent.turn().await.unwrap();
    assert_eq!(absent.calls().await, 0);
    let f = fixture(
        enabled(json!({"retitle": false})),
        &[r#"{"title":"First topic title"}"#],
        false,
    )
    .await;
    f.turn().await.unwrap();
    f.inputs(10).await;
    f.age().await;
    f.turn().await.unwrap();
    assert_eq!(f.calls().await, 1);
}

#[tokio::test]
async fn explicit_same_text_takes_ownership() {
    let f = fixture(
        enabled(json!({})),
        &[r#"{"title":"First topic title"}"#],
        false,
    )
    .await;
    f.turn().await.unwrap();
    f.db.update_session(
        1,
        f.id,
        UpdateSession {
            title: Some("First topic title".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    f.inputs(32).await;
    f.age().await;
    f.turn().await.unwrap();
    assert_eq!(f.calls().await, 1);
    assert!(
        !f.db
            .automatic_title_state(1, f.id)
            .await
            .unwrap()
            .title_is_automatic
    );
}

#[tokio::test]
async fn explicit_rename_wins_in_flight_generation() {
    let f = fixture(
        enabled(json!({})),
        &[r#"{"title":"Automatic topic title"}"#],
        true,
    )
    .await;
    let listener = f.listener.clone();
    let id = f.id;
    let task = tokio::spawn(async move { listener.generate_for_turn(id, 10).await });
    let (started, release) = f.generator.pause.as_ref().unwrap();
    started.notified().await;
    f.db.update_session(
        1,
        f.id,
        UpdateSession {
            title: Some("My chosen title".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    release.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(f.title().await.as_deref(), Some("My chosen title"));
}

#[tokio::test]
async fn concurrent_delivery_and_failures_preserve_reservation() {
    let f = fixture(enabled(json!({})), &["invalid JSON"], false).await;
    let other = AutomaticSessionTitlesListener::new(
        f.db.clone(),
        SessionService::new(f.db.clone()),
        f.generator.clone(),
    );
    other.bind_events(&f.events);
    let (a, b) = tokio::join!(
        f.listener.generate_for_turn(f.id, 10),
        other.generate_for_turn(f.id, 10)
    );
    assert!(a.is_err() || b.is_err());
    assert_eq!(f.calls().await, 1);
    f.inputs(7).await;
    f.age().await;
    assert!(
        f.db.claim_automatic_title_check(1, f.id, 20, true)
            .await
            .unwrap()
            .is_none()
    );
    f.inputs(1).await;
    assert!(
        f.db.claim_automatic_title_check(1, f.id, 20, true)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        f.db.set_automatic_title(1, f.id, Some("Old generation result"), 15, true)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.db.set_automatic_title(2, f.id, Some("Foreign tenant title"), 20, true)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        f.db.set_automatic_title(1, f.id, Some("New generation result"), 20, true)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        f.db.set_automatic_title(1, f.id, Some("Old generation result"), 10, true)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn title_and_semantic_event_roll_back_together() {
    let f = fixture(
        enabled(json!({})),
        &[r#"{"title":"Plan database migration"}"#],
        false,
    )
    .await;
    f.db.test_database_handle()
        .unwrap()
        .hooks()
        .force_failure("create_event");
    assert!(f.listener.generate_for_turn(f.id, 10).await.is_err());
    let state = f.db.automatic_title_state(1, f.id).await.unwrap();
    assert!(state.title.is_none());
    assert_eq!(state.auto_title_sequence, 0);
    assert_eq!(state.auto_title_attempt_sequence, 10);
}

#[tokio::test]
async fn title_requests_do_not_block_turn_completion() {
    let f = fixture(
        enabled(json!({})),
        &[r#"{"title":"Plan database migration"}"#],
        true,
    )
    .await;
    let listener = Arc::new(f.listener.clone());
    let events =
        EventService::with_listeners(f.db.clone(), EventDelivery::in_memory(), vec![listener]);
    tokio::time::timeout(
        Duration::from_secs(2),
        events.emit(EventRequest::new(
            f.id,
            EventContext::empty(),
            TurnCompletedData::default(),
        )),
    )
    .await
    .unwrap()
    .unwrap();
    let (started, release) = f.generator.pause.as_ref().unwrap();
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if f.title().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn small_default_is_live_and_explicit_modes_override_it() {
    for mode in ["default", "session", "pinned"] {
        let pinned = ModelId::new();
        let config = match mode {
            "session" => json!({"model_id":"session"}),
            "pinned" => json!({"model_id":pinned}),
            _ => json!({}),
        };
        let f = fixture(
            enabled(config),
            &[r#"{"title":"Plan database migration"}"#],
            false,
        )
        .await;
        let small = f.small().await;
        f.turn().await.unwrap();
        let expected = match mode {
            "session" => f.model,
            "pinned" => pinned,
            _ => small,
        };
        assert_eq!(*f.generator.models.lock().await, vec![expected]);
        assert_eq!(
            f.db.get_session(1, f.id).await.unwrap().unwrap().model_id,
            Some(f.model)
        );
    }
}

#[tokio::test]
async fn unavailable_small_selection_does_not_fall_back() {
    let f = fixture(enabled(json!({})), &[], false).await;
    let small = f.small().await;
    f.db.update_model(
        1,
        small.uuid(),
        crate::storage::UpdateModel {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(f.turn().await.is_err());
    assert_eq!(*f.generator.models.lock().await, vec![small]);
    assert!(f.title().await.is_none());
}

#[tokio::test]
async fn real_generator_rejects_missing_and_foreign_models_before_http() {
    let f = fixture(enabled(json!({})), &[], false).await;
    let generator = LlmTitleGenerator {
        db: f.db.clone(),
        resolver: Arc::new(ProviderResolverService::new(f.db.clone(), None)),
        drivers: Arc::new(DriverRegistry::new()),
    };
    assert!(
        generator
            .generate(2, f.id, f.model, "data".into())
            .await
            .is_err()
    );
    assert!(
        generator
            .generate(1, f.id, ModelId::new(), "data".into())
            .await
            .is_err()
    );
}

#[test]
fn strict_bounded_title_response() {
    assert_eq!(parse_title(r#"{"title":null}"#).unwrap(), None);
    assert_eq!(
        parse_title(r#"{"title":"  План подорожі Україною  "}"#)
            .unwrap()
            .as_deref(),
        Some("План подорожі Україною")
    );
    for text in [
        "{}",
        "hello",
        r#"{"title":1}"#,
        r#"{"title":""}"#,
        r#"{"title":"bad\nvalue"}"#,
        r#"{"title":null,"extra":true}"#,
    ] {
        assert!(parse_title(text).is_err(), "{text}");
    }
    assert!(parse_title(&json!({"title":"x".repeat(101)}).to_string()).is_err());
}

#[tokio::test]
async fn manual_transaction_does_not_deadlock_with_advisory_write() {
    let f = fixture(enabled(json!({})), &[], false).await;
    assert!(
        f.db.claim_automatic_title_check(1, f.id, 10, true)
            .await
            .unwrap()
            .is_some()
    );
    let mut manual = f.db.pool().begin().await.unwrap();
    sqlx::query(
        "UPDATE sessions SET title = 'Explicit title', title_is_automatic = FALSE WHERE id = $1",
    )
    .bind(f.id)
    .execute(&mut *manual)
    .await
    .unwrap();
    // The manual command holds sessions and may next emit an event, which
    // locks event_sequences. Advisory work must yield instead of waiting back.
    let automatic = tokio::time::timeout(
        Duration::from_secs(2),
        f.db.set_automatic_title(1, f.id, Some("Automatic title"), 10, true),
    )
    .await
    .unwrap();
    assert!(automatic.is_err());
    manual.commit().await.unwrap();
    assert_eq!(f.title().await.as_deref(), Some("Explicit title"));
}

#[tokio::test]
async fn inherited_capability_and_session_override_share_runtime_resolution() {
    let f = fixture(
        json!([]),
        &[r#"{"title":"Plan database migration"}"#],
        false,
    )
    .await;
    let harness =
        f.db.get_session(1, f.id)
            .await
            .unwrap()
            .unwrap()
            .harness_id
            .unwrap();
    f.db.set_harness_capabilities(
        harness.uuid(),
        vec![(CAPABILITY_ID.into(), 0, json!({"retitle":false}))],
    )
    .await
    .unwrap();
    f.turn().await.unwrap();
    f.inputs(32).await;
    f.age().await;
    f.turn().await.unwrap();
    assert_eq!(f.calls().await, 1);
    assert_eq!(f.title().await.as_deref(), Some("Plan database migration"));
}

#[tokio::test]
async fn transcript_projection_is_tenant_scoped_bounded_and_text_only() {
    let f = fixture(enabled(json!({})), &[], false).await;
    f.events
        .emit(EventRequest::new(
            f.id,
            EventContext::empty(),
            InputMessageData::new(RuntimeMessage::user("🦊".repeat(100_000))),
        ))
        .await
        .unwrap();
    let messages =
        f.db.automatic_title_messages(1, f.id, i32::MAX)
            .await
            .unwrap();
    assert_eq!(messages[0].1.as_ref().unwrap().chars().count(), 2000);
    assert!(
        f.db.automatic_title_messages(2, f.id, i32::MAX)
            .await
            .unwrap()
            .is_empty()
    );
    let input = title_input(None, &messages).unwrap();
    assert!(input.len() < MAX_INPUT_BYTES + 1000);
    let input: serde_json::Value = serde_json::from_str(&input).unwrap();
    assert!(
        input["transcript"]
            .as_array()
            .unwrap()
            .iter()
            .all(|message| message["text"].as_str().unwrap().len() <= MAX_MESSAGE_BYTES)
    );
    assert!(title_input(None, &[("input.message".into(), None)]).is_none());
}
