//! A v2 task's webhooks (`task_webhooks.rs`), on a real database: a
//! transition the feed carries goes to the org's task webhooks (terminal
//! only) and to the push configs the record names, signed, once per feed
//! position.

use everruns_contracts::dataplane::PushTarget;
use everruns_core::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};

use super::*;

/// Keeps what it is asked to send.
#[derive(Default)]
struct Recording(Mutex<Vec<EgressRequest>>);

#[async_trait::async_trait]
impl EgressService for Recording {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        self.0.lock().push(request);
        Err(EgressError::Transport("recorded".into()))
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        self.0.lock().push(request);
        Err(EgressError::Transport("recorded".into()))
    }
}

const SESSION: &str = "session_01933b5a000070008000000000000001";

fn transition(index: u64, transition: &str, targets: Vec<PushTarget>) -> FeedRecord {
    FeedRecord {
        index,
        ordinal: 0,
        org_id: org_public(ORG),
        at_unix_ms: chrono::Utc::now().timestamp_millis(),
        record: Record::TaskTransition {
            session_id: SESSION.into(),
            transition: transition.into(),
            task: json!({ "id": "task-1", "state": "succeeded", "kind": "subagent" }),
            targets,
        },
    }
}

async fn sent(egress: &Recording, count: usize) -> Vec<EgressRequest> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let seen = egress.0.lock().clone();
        if seen.len() >= count || Instant::now() > deadline {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_v2_tasks_transitions_reach_its_webhooks_once() {
    let (cp, _, _) = control_plane().await;
    let egress = Arc::new(Recording::default());
    let cp = cp.with_egress(egress.clone());
    cp.db
        .create_org_task_webhook(crate::storage::CreateOrgTaskWebhook {
            public_id: "twh_org".into(),
            org_id: ORG,
            url: "https://hooks.example.test/org".into(),
            secret: None,
            enabled: true,
        })
        .await
        .unwrap();
    // The config's secret travels sealed with the org's data key, as the
    // edge that took it sealed it.
    let now = chrono::Utc::now();
    cp.db
        .dp_upsert_session_summary(crate::storage::DpSessionSummary {
            session_id: SESSION,
            org_id: ORG,
            data_plane_id: "dp-1",
            agent_id: "agent",
            session_version: 0,
            title: "",
            status: "started",
            turns: 0,
            cost_micros: 0,
            created_at: now,
            last_activity_at: now,
        })
        .await
        .unwrap();
    let row = cp.db.dp_org(ORG).await.unwrap().unwrap();
    let key = cp.data_key(&row).unwrap();
    let sealed_for = |config: &str| {
        everruns_contracts::dataplane_seal::seal(
            &key,
            &row.data_key_id,
            &org_public(ORG),
            &everruns_contracts::dataplane_seal::push_secret_id(SESSION, config),
            0,
            b"k",
            everruns_contracts::dataplane_seal::random_nonce(),
        )
        .unwrap()
    };
    let push = PushTarget {
        label: "tpc_a".into(),
        url: "https://hooks.example.test/push".into(),
        secret: None,
        sealed: Some(sealed_for("tpc_a")),
    };
    // A secret sealed for another config does not open here, and the
    // target is not sent unsigned.
    let lost = PushTarget {
        label: "tpc_lost".into(),
        url: "https://hooks.example.test/lost".into(),
        secret: None,
        sealed: Some(sealed_for("tpc_a")),
    };
    let budgets = BudgetService::new(cp.db.clone());
    let source = format!("shard-tasks-{}", uuid::Uuid::new_v4().simple());
    let batch = FeedBatch {
        source: source.clone(),
        data_plane: "dp-1".into(),
        incarnation: 1,
        records: vec![
            transition(1, "terminal", vec![push.clone(), lost]),
            // Waiting for input: the org's webhooks never fire on it.
            transition(2, "awaiting_input", vec![push.clone()]),
        ],
    };
    feed::ingest(&cp, &budgets, &[], batch.clone())
        .await
        .unwrap();
    // A repeated batch delivers nothing again.
    feed::ingest(&cp, &budgets, &[], batch).await.unwrap();
    let requests = sent(&egress, 3).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let requests_after = egress.0.lock().clone();
    assert_eq!(
        requests_after.len(),
        3,
        "{:?}",
        requests_after.iter().map(|r| &r.url).collect::<Vec<_>>()
    );
    let urls: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(
        urls.iter().filter(|u| u.ends_with("/org")).count(),
        1,
        "{urls:?}"
    );
    assert_eq!(urls.iter().filter(|u| u.ends_with("/push")).count(), 2);
    for request in &requests {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["task"]["id"], "task-1");
        assert!(body["event"].as_str().unwrap().starts_with("task."));
        // Signed when the target has a secret.
        assert_eq!(
            request.headers.contains_key("X-Everruns-Signature"),
            request.url.ends_with("/push"),
        );
    }
}
