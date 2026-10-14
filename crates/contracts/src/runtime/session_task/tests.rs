use super::*;

fn instant(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn snapshot(task: &SessionTask) -> Value {
    serde_json::to_value(task).unwrap()
}

fn task() -> SessionTask {
    new_session_task(
        CreateSessionTask {
            session_id: SessionId::from_uuid(uuid::Uuid::from_u128(1)),
            id: Some("task_fixed".into()),
            kind: TASK_KIND_BACKGROUND_TOOL.to_string(),
            display_name: "Test".to_string(),
            spec: serde_json::json!({}),
            state: SessionTaskState::Queued,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        },
        instant(10),
    )
}

#[test]
fn recovery_guard_rejects_every_changed_ownership_or_activity_field() {
    let mut observed = task();
    apply_task_update(
        &mut observed,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            worker_id: Some("owner".into()),
            heartbeat_at: Some(instant(11)),
            ..Default::default()
        },
        instant(11),
    );
    let guard = SessionTaskRecoveryGuard::from_task(&observed);
    let mut changes = Vec::new();
    let mut changed = observed.clone();
    changed.state = SessionTaskState::Succeeded;
    changes.push(changed);
    let mut changed = observed.clone();
    changed.attempt += 1;
    changes.push(changed);
    let mut changed = observed.clone();
    changed.worker_id = Some("new-owner".into());
    changes.push(changed);
    let mut changed = observed.clone();
    changed.heartbeat_at = Some(instant(12));
    changes.push(changed);
    let mut changed = observed.clone();
    changed.cancel_requested_at = Some(instant(12));
    changes.push(changed);
    let mut changed = observed.clone();
    changed.updated_at = instant(12);
    changes.push(changed);
    for mut changed in changes {
        let before = snapshot(&changed);
        apply_task_update(
            &mut changed,
            SessionTaskUpdate {
                expected_recovery: Some(guard.clone()),
                state: Some(SessionTaskState::Failed),
                summary: Some("stale recovery".into()),
                ..Default::default()
            },
            instant(13),
        );
        assert_eq!(
            snapshot(&changed),
            before,
            "stale recovery changed the task"
        );
    }
}

#[test]
fn recovery_guard_accepts_the_exact_observed_snapshot() {
    let mut observed = task();
    apply_task_update(
        &mut observed,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        },
        instant(11),
    );
    let guard = SessionTaskRecoveryGuard::from_task(&observed);
    apply_task_update(
        &mut observed,
        SessionTaskUpdate {
            expected_recovery: Some(guard),
            state: Some(SessionTaskState::Failed),
            ..Default::default()
        },
        instant(12),
    );
    assert_eq!(observed.state, SessionTaskState::Failed);
    assert_eq!(observed.finished_at, Some(instant(12)));
}

#[test]
fn creation_preserves_inputs_and_initial_lifecycle_timestamps() {
    for (state, started, finished) in [
        (SessionTaskState::Queued, None, None),
        (SessionTaskState::Running, Some(instant(10)), None),
        (SessionTaskState::AwaitingInput, Some(instant(10)), None),
        (
            SessionTaskState::Succeeded,
            Some(instant(10)),
            Some(instant(10)),
        ),
        (
            SessionTaskState::Failed,
            Some(instant(10)),
            Some(instant(10)),
        ),
        (
            SessionTaskState::Canceled,
            Some(instant(10)),
            Some(instant(10)),
        ),
    ] {
        let input = CreateSessionTask {
            session_id: task().session_id,
            id: Some("task_external".into()),
            kind: "custom_kind".into(),
            display_name: "Work".into(),
            spec: serde_json::json!({"input": [1, 2]}),
            state,
            links: TaskLinks {
                remote_task_id: Some("remote".into()),
                ..Default::default()
            },
            wake_policy: TaskWakePolicy::OnActivity,
        };
        let actual = new_session_task(input.clone(), instant(10));
        let mut expected = task();
        expected.id = "task_external".into();
        expected.kind = "custom_kind".into();
        expected.display_name = "Work".into();
        expected.spec = serde_json::json!({"input": [1, 2]});
        expected.state = state;
        expected.links.remote_task_id = Some("remote".into());
        expected.wake_policy = TaskWakePolicy::OnActivity;
        expected.started_at = started;
        expected.finished_at = finished;
        assert_eq!(snapshot(&actual), snapshot(&expected));
        let generated = new_session_task(CreateSessionTask { id: None, ..input }, instant(10));
        let suffix = generated.id.strip_prefix("task_").unwrap();
        assert_eq!(suffix.len(), 32);
        assert_eq!(uuid::Uuid::parse_str(suffix).unwrap().get_version_num(), 7);
    }
}

#[test]
fn serialization_redacts_spec_push_config_secrets() {
    let mut t = task();
    t.spec = serde_json::json!({
        "instructions": "notify",
        "push_configs": [
            {
                "url": "https://hooks.example.com/everruns",
                "secret": "LEAKME-HMAC-KEY",
                "event_filter": ["terminal"]
            },
            {
                "url": "https://hooks.example.com/no-secret",
                "event_filter": ["message"]
            }
        ]
    });

    let stored = t.spec.clone();
    assert_eq!(
        snapshot(&t)["spec"],
        serde_json::json!({
            "instructions": "notify",
            "push_configs": [
                {"url": "https://hooks.example.com/everruns", "has_secret": true, "event_filter": ["terminal"]},
                {"url": "https://hooks.example.com/no-secret", "event_filter": ["message"]}
            ]
        })
    );
    assert_eq!(
        t.spec, stored,
        "presentation must not mutate delivery secrets"
    );
}

#[test]
fn first_transition_out_of_queued_stamps_started_at() {
    let mut t = task();
    let now = instant(20);
    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        },
        now,
    );
    assert_eq!(t.state, SessionTaskState::Running);
    assert_eq!(t.started_at, Some(now));
    assert!(t.finished_at.is_none());
    assert_eq!(t.updated_at, instant(20));
    for (state, at) in [
        (SessionTaskState::Queued, 30),
        (SessionTaskState::Running, 40),
    ] {
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(state),
                ..Default::default()
            },
            instant(at),
        );
        assert_eq!(
            t.started_at,
            Some(instant(20)),
            "first start must survive requeue"
        );
        assert_eq!(t.updated_at, instant(at));
    }
}

#[test]
fn terminal_transitions_reject_conflicting_updates_but_allow_enrichment() {
    use SessionTaskState::*;
    for terminal in [Succeeded, Failed, Canceled] {
        let mut t = task();
        apply_task_update(
            &mut t,
            SessionTaskUpdate {
                state: Some(terminal),
                summary: Some("done".into()),
                ..Default::default()
            },
            instant(20),
        );
        assert_eq!(t.state, terminal);
        assert_eq!(t.started_at, Some(instant(20)));
        assert_eq!(t.finished_at, Some(instant(20)));
        assert_eq!(t.updated_at, instant(20));
        let before = snapshot(&t);
        for other in [Queued, Running, AwaitingInput, Succeeded, Failed, Canceled] {
            if other == terminal {
                continue;
            }
            apply_task_update(
                &mut t,
                SessionTaskUpdate {
                    state: Some(other),
                    summary: Some("stale".into()),
                    error: Some(TaskError {
                        kind: "orphaned".into(),
                        message: "stale".into(),
                    }),
                    append_artifact: Some(artifact("stale")),
                    increment_attempt: true,
                    ..Default::default()
                },
                instant(30),
            );
            assert_eq!(snapshot(&t), before, "{terminal:?} -> {other:?}");
        }
        let mut expected = t.clone();
        for state in [Some(terminal), None] {
            apply_task_update(
                &mut t,
                SessionTaskUpdate {
                    state,
                    result_path: Some("/result".into()),
                    summary: Some("enriched".into()),
                    input_request: Some(TaskInputRequest {
                        id: "late".into(),
                        prompt: "too late".into(),
                        expected: None,
                    }),
                    ..Default::default()
                },
                instant(40),
            );
            expected.result_path = Some("/result".into());
            expected.summary = Some("enriched".into());
            expected.updated_at = instant(40);
            assert_eq!(
                snapshot(&t),
                snapshot(&expected),
                "enrichment must preserve lifecycle and ignore late input"
            );
        }
    }
}

#[test]
fn input_request_forces_awaiting_input_and_clears_on_resume() {
    let mut t = task();
    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            input_request: Some(TaskInputRequest {
                id: "req_1".to_string(),
                prompt: "Approve?".to_string(),
                expected: None,
            }),
            ..Default::default()
        },
        instant(10),
    );
    assert_eq!(t.state, SessionTaskState::AwaitingInput);
    assert_eq!(
        t.input_request,
        Some(TaskInputRequest {
            id: "req_1".into(),
            prompt: "Approve?".into(),
            expected: None
        })
    );
    assert_eq!(t.started_at, Some(instant(10)));

    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        },
        instant(10),
    );
    assert_eq!(t.state, SessionTaskState::Running);
    assert!(t.input_request.is_none());
}

#[test]
fn links_merge_without_duplicates() {
    let mut t = task();
    let child = SessionId::from_uuid(uuid::Uuid::from_u128(1));
    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            links: Some(TaskLinks {
                child_session_id: Some(child),
                remote_task_id: None,
                resource_ids: vec!["res_1".to_string()],
            }),
            ..Default::default()
        },
        instant(10),
    );
    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            links: Some(TaskLinks {
                child_session_id: None,
                remote_task_id: Some("rt_1".to_string()),
                resource_ids: vec!["res_1".to_string(), "res_2".to_string()],
            }),
            ..Default::default()
        },
        instant(10),
    );
    assert_eq!(t.links.child_session_id, Some(child));
    assert_eq!(t.links.remote_task_id.as_deref(), Some("rt_1"));
    assert_eq!(t.links.resource_ids, vec!["res_1", "res_2"]);
    let replacement = SessionId::from_uuid(uuid::Uuid::from_u128(2));
    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            links: Some(TaskLinks {
                child_session_id: Some(replacement),
                remote_task_id: Some("rt_2".into()),
                resource_ids: vec!["res_2".into(), "res_3".into(), "res_3".into()],
            }),
            ..Default::default()
        },
        instant(30),
    );
    assert_eq!(
        t.links,
        TaskLinks {
            child_session_id: Some(replacement),
            remote_task_id: Some("rt_2".into()),
            resource_ids: vec!["res_1".into(), "res_2".into(), "res_3".into()]
        }
    );
}

#[test]
fn message_text_rendering() {
    let content = vec![
        TaskMessagePart::Data {
            data: serde_json::json!({"text": "hidden"}),
        },
        TaskMessagePart::text("first\nline"),
        TaskMessagePart::text(""),
        TaskMessagePart::Data {
            data: serde_json::json!([1, 2]),
        },
        TaskMessagePart::text("last 🦀"),
    ];
    assert_eq!(task_message_text(&content), "first\nline\n\nlast 🦀");
    assert_eq!(task_message_text(&[]), "");
    assert_eq!(task_message_text(&content[..1]), "");
}

// -------------------------------------------------------------------------
// Stale-attempt fencing tests
// -------------------------------------------------------------------------

#[test]
fn attempt_fence_rejects_entire_update_and_allows_current_or_unfenced_writes() {
    for expected_attempt in [Some(1), Some(2), Some(3), None] {
        let mut actual = task();
        actual.attempt = 2;
        actual.artifacts = vec![artifact("old")];
        let before = snapshot(&actual);
        let update = SessionTaskUpdate {
            state: Some(SessionTaskState::Running),
            state_detail: Some("working".into()),
            summary: Some("summary".into()),
            result_path: Some("/result".into()),
            artifacts: Some(vec![artifact("replacement")]),
            append_artifact: Some(artifact("append")),
            error: Some(TaskError {
                kind: "diagnostic".into(),
                message: "detail".into(),
            }),
            links: Some(TaskLinks {
                remote_task_id: Some("remote".into()),
                ..Default::default()
            }),
            worker_id: Some("worker".into()),
            heartbeat_at: Some(instant(19)),
            expected_attempt,
            increment_attempt: true,
            ..Default::default()
        };
        let mut expected = actual.clone();
        apply_task_update(&mut actual, update, instant(20));
        if matches!(expected_attempt, Some(1 | 3)) {
            assert_eq!(
                snapshot(&actual),
                before,
                "stale/future attempt {expected_attempt:?}"
            );
        } else {
            expected.state = SessionTaskState::Running;
            expected.state_detail = Some("working".into());
            expected.summary = Some("summary".into());
            expected.result_path = Some("/result".into());
            expected.artifacts = vec![artifact("replacement"), artifact("append")];
            expected.error = Some(TaskError {
                kind: "diagnostic".into(),
                message: "detail".into(),
            });
            expected.links.remote_task_id = Some("remote".into());
            expected.worker_id = Some("worker".into());
            expected.heartbeat_at = Some(instant(19));
            expected.started_at = Some(instant(20));
            expected.updated_at = instant(20);
            expected.attempt = 3;
            assert_eq!(snapshot(&actual), snapshot(&expected));
            apply_task_update(
                &mut actual,
                SessionTaskUpdate {
                    artifacts: Some(vec![]),
                    ..Default::default()
                },
                instant(30),
            );
            assert!(
                actual.artifacts.is_empty(),
                "explicit empty replacement clears artifacts"
            );
        }
    }
}

#[test]
fn reaper_update_increments_attempt_and_fences_old_executor() {
    let mut t = task();
    t.state = SessionTaskState::Running;
    assert_eq!(t.attempt, 1);
    let now = instant(20);

    // Reaper-style update: fail as orphaned and supersede the attempt.
    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            state: Some(SessionTaskState::Failed),
            error: Some(TaskError {
                kind: "orphaned".to_string(),
                message: "worker heartbeat stopped".to_string(),
            }),
            increment_attempt: true,
            ..Default::default()
        },
        now,
    );
    assert_eq!(t.state, SessionTaskState::Failed);
    assert_eq!(t.attempt, 2, "orphan reap must supersede the attempt");

    assert_eq!(
        t.error,
        Some(TaskError {
            kind: "orphaned".into(),
            message: "worker heartbeat stopped".into()
        })
    );
    assert_eq!(t.finished_at, Some(instant(20)));
    let before = snapshot(&t);
    apply_task_update(
        &mut t,
        SessionTaskUpdate {
            heartbeat_at: Some(instant(30)),
            append_artifact: Some(artifact("zombie")),
            expected_attempt: Some(1),
            ..Default::default()
        },
        instant(30),
    );
    assert_eq!(snapshot(&t), before);
}

struct ArtifactRegistry {
    task: tokio::sync::Mutex<SessionTask>,
}

#[async_trait]
impl SessionTaskRegistry for ArtifactRegistry {
    async fn create(&self, _input: CreateSessionTask) -> Result<SessionTask> {
        panic!("unexpected create")
    }
    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        let mut task = self.task.lock().await;
        assert_eq!(task.session_id, session_id);
        assert_eq!(task.id, task_id);
        apply_task_update(&mut task, update, instant(10));
        Ok(Some(task.clone()))
    }
    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        let task = self.task.lock().await.clone();
        assert_eq!(task.session_id, session_id);
        assert_eq!(task.id, task_id);
        // Force concurrent read/modify/write callers to observe the same snapshot.
        tokio::task::yield_now().await;
        Ok(Some(task))
    }
    async fn list(
        &self,
        _session_id: SessionId,
        _filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        panic!("unexpected list")
    }
    async fn request_cancel(
        &self,
        _session_id: SessionId,
        _task_id: &str,
    ) -> Result<Option<SessionTask>> {
        panic!("unexpected cancel")
    }
    async fn record_message(
        &self,
        _session_id: SessionId,
        _task_id: &str,
        _message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        panic!("unexpected message")
    }
    async fn list_messages(
        &self,
        _session_id: SessionId,
        _task_id: &str,
        _limit: Option<u32>,
        _after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        panic!("unexpected messages")
    }
}

fn artifact(name: &str) -> TaskArtifact {
    TaskArtifact {
        name: name.into(),
        artifact_type: "file".into(),
        path: Some(format!("/results/{name}")),
        url: None,
    }
}

#[tokio::test]
async fn concurrent_sinks_append_artifacts_without_losing_siblings() {
    let mut task = task();
    task.artifacts.push(artifact("initial"));
    let session_id = task.session_id;
    let task_id = task.id.clone();
    let registry = Arc::new(ArtifactRegistry {
        task: tokio::sync::Mutex::new(task),
    });
    let first = RegistryTaskSink::new(registry.clone(), session_id, task_id.clone());
    let second = RegistryTaskSink::new(registry.clone(), session_id, task_id);
    let (a, b) = tokio::join!(
        first.artifact(artifact("a")),
        second.artifact(artifact("b"))
    );
    a.unwrap();
    b.unwrap();
    let mut artifacts = registry.task.lock().await.artifacts.clone();
    artifacts.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        artifacts,
        [artifact("a"), artifact("b"), artifact("initial")]
    );
    registry.task.lock().await.attempt = 2;
    let before = snapshot(&*registry.task.lock().await);
    first.artifact(artifact("stale")).await.unwrap();
    assert_eq!(snapshot(&*registry.task.lock().await), before);
}
