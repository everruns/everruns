use super::*;
use crate::storage::{CreateSessionRow, DbSessionTaskRegistry};
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::session_task::{CreateSessionTask, TaskLinks, TaskWakePolicy};

struct Fixture {
    db: Arc<StorageBackend>,
    registry: DbSessionTaskRegistry,
    coordinator: SessionId,
    thread: SessionId,
    task_id: String,
}

async fn fixture() -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let coordinator = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("chat".to_string()),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    let thread = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: PrincipalId::from_seed(1),
            title: Some("thread".to_string()),
            parent_session_id: Some(coordinator),
            source: crate::records::SessionSource::Subagent,
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    let registry = DbSessionTaskRegistry::new(db.clone());
    let task_id = registry
        .create(CreateSessionTask {
            session_id: coordinator,
            id: None,
            kind: TASK_KIND_ASSIGNMENT.to_string(),
            display_name: "Work".to_string(),
            spec: serde_json::json!({}),
            state: SessionTaskState::Running,
            links: TaskLinks {
                child_session_id: Some(thread),
                ..Default::default()
            },
            wake_policy: TaskWakePolicy::OnActivity,
        })
        .await
        .unwrap()
        .id;
    Fixture {
        db,
        registry,
        coordinator,
        thread,
        task_id,
    }
}

impl Fixture {
    async fn settle(&self, event_type: &str) {
        settle_thread_turn(&self.db, &self.registry, self.thread, event_type)
            .await
            .unwrap();
    }

    async fn state(&self) -> SessionTaskState {
        self.registry
            .get(self.coordinator, &self.task_id)
            .await
            .unwrap()
            .unwrap()
            .state
    }
}

#[tokio::test]
async fn a_turn_that_ends_without_completing_needs_attention_and_the_next_turn_resumes() {
    let fixture = fixture().await;
    fixture.settle(TURN_COMPLETED).await;
    let task = fixture
        .registry
        .get(fixture.coordinator, &fixture.task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.state, SessionTaskState::AwaitingInput);
    assert!(
        task.input_request
            .unwrap()
            .prompt
            .contains("without completing the assignment")
    );

    fixture.settle(TURN_STARTED).await;
    assert_eq!(fixture.state().await, SessionTaskState::Running);
}

#[tokio::test]
async fn a_failed_turn_fails_the_assignment() {
    let fixture = fixture().await;
    fixture.settle(TURN_FAILED).await;
    assert_eq!(fixture.state().await, SessionTaskState::Failed);
}

#[tokio::test]
async fn completed_assignments_and_unrelated_sessions_are_left_alone() {
    let fixture = fixture().await;
    fixture
        .registry
        .update(
            fixture.coordinator,
            &fixture.task_id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    fixture.settle(TURN_COMPLETED).await;
    assert_eq!(fixture.state().await, SessionTaskState::Succeeded);

    // The coordinator itself has no parent: nothing to settle.
    settle_thread_turn(
        &fixture.db,
        &fixture.registry,
        fixture.coordinator,
        TURN_FAILED,
    )
    .await
    .unwrap();
    assert_eq!(fixture.state().await, SessionTaskState::Succeeded);
}
