//! `changes_only` trigger filter, end to end through webhook ingress.

use super::*;

fn ci_event(delivery: &str, branch: &str, outcome: &str) -> WebhookTriggerInvocationRequest {
    let payload = serde_json::json!({"branch": branch, "outcome": outcome});
    WebhookTriggerInvocationRequest {
        ingress_id: String::new(),
        body: payload.to_string(),
        json_payload: Some(payload),
        headers: [("x-delivery".to_string(), delivery.to_string())]
            .into_iter()
            .collect(),
    }
}

#[tokio::test]
async fn changes_only_wakes_when_a_subject_flips() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db.clone(), OrgRole::Owner);
    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: CreateAgentTriggerRequest {
            message: "CI on {{payload.branch}} is {{payload.outcome}}".to_string(),
            event_id_template: Some("{{webhook.headers.x-delivery}}".to_string()),
            subject_template: Some("{{payload.branch}}".to_string()),
            filter: Some(crate::domains::agent_triggers::record::TriggerEventFilter {
                conditions: vec![],
                changes_only: Some(crate::domains::agent_triggers::record::TriggerChangesOnly {
                    path: "payload.outcome".to_string(),
                }),
            }),
            ..webhook_req(true)
        },
    }
    .run(&ctx)
    .await
    .expect("create webhook trigger");
    let ingress_id = trigger.ingress_id.expect("webhook ingress").to_string();

    let runner = Arc::new(RecordingRunner::default());
    let session_service = SessionService::new(db.clone());
    let message_service =
        MessageService::new(db.clone(), runner.clone(), EventDelivery::in_memory());
    let encryption = ctx.encryption.clone();
    let fire = async |event: WebhookTriggerInvocationRequest| {
        let outcome = invoke_webhook_agent_trigger(
            &db,
            encryption.as_ref(),
            &session_service,
            &message_service,
            WebhookTriggerInvocationRequest {
                ingress_id: ingress_id.clone(),
                ..event
            },
            None,
        )
        .await
        .expect("event handled");
        match outcome {
            events::TriggerEventOutcome::Dispatched(_) => "dispatched",
            events::TriggerEventOutcome::Filtered => "filtered",
            events::TriggerEventOutcome::Duplicate => "duplicate",
        }
    };

    // First value per subject only records; repeats stay quiet; flips wake.
    assert_eq!(fire(ci_event("1", "main", "success")).await, "filtered");
    assert_eq!(fire(ci_event("2", "main", "success")).await, "filtered");
    assert_eq!(fire(ci_event("3", "main", "failure")).await, "dispatched");
    assert_eq!(fire(ci_event("3", "main", "failure")).await, "duplicate");
    assert_eq!(fire(ci_event("4", "main", "failure")).await, "filtered");
    assert_eq!(fire(ci_event("5", "dev", "failure")).await, "filtered");
    assert_eq!(fire(ci_event("6", "main", "success")).await, "dispatched");

    let reasons: Vec<_> = ListAgentTriggerDeliveries {
        agent_id,
        trigger_id: trigger.id.to_string(),
        limit: None,
    }
    .run(&ctx)
    .await
    .expect("list deliveries")
    .into_iter()
    .rev()
    .filter_map(|delivery| delivery.reason)
    .collect();
    assert_eq!(
        reasons,
        vec![
            "first payload.outcome for this subject is success",
            "payload.outcome is still success",
            "payload.outcome is still failure",
            "first payload.outcome for this subject is failure",
        ]
    );
}
