use super::*;
use everruns_contracts::{
    tool_types::ToolCall,
    typed_id::{MessageId, TurnId},
};
use everruns_core::{
    events::{ToolCallRequestedData, TurnCancelledData, TurnStartedData},
    human_wait::METADATA_KEY,
    message::ContentPart,
};
use serde_json::{Value, json};

#[tokio::test]
async fn human_wait_batch_partial_resolution_restart_and_late_completion() {
    let db = Arc::new(StorageBackend::test_database());
    let service = EventService::new(db.clone(), EventDelivery::in_memory());
    let agent = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);
    let session = db
        .create_session(test_session_input(agent))
        .await
        .unwrap()
        .id;
    let turn = TurnId::new();
    let input = MessageId::new();
    let context = EventContext::turn(turn, input);
    let start = EventRequest::new(
        session,
        context.clone(),
        TurnStartedData {
            turn_id: turn,
            input_message_id: input,
            input_content: None,
            agent_id: None,
            agent_name: None,
            agent_description: None,
            origin: None,
        },
    );
    let mut ask = EventRequest::new(
        session,
        context.clone(),
        ToolCallRequestedData::with_definitions(
            &[
                ToolCall {
                    id: "q".into(),
                    name: "ask_user".into(),
                    arguments: json!({"question":"PRIVATE_PROMPT"}),
                },
                ToolCall {
                    id: "a".into(),
                    name: everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL.into(),
                    arguments: json!({"url":"https://private.example", "password":"PRIVATE_KEY"}),
                },
            ],
            &[],
        ),
    );
    ask.ts = chrono::Utc::now() - chrono::Duration::seconds(30);
    let asked_at = ask.ts;
    let answer = EventRequest::new(
        session,
        context.clone(),
        ToolCompletedData::success(
            "q".into(),
            "ask_user".into(),
            vec![ContentPart::tool_result_text(
                &json!({"status":"answered","answered_by":"user","answers":["PRIVATE_ANSWER"]}),
            )],
            None,
        ),
    );
    // All three can arrive together; the request is not in PostgreSQL yet.
    let (_, last) = service
        .emit_batch_returning_last(vec![start, ask, answer])
        .await
        .unwrap();
    let answered = last.unwrap();
    assert_eq!(
        answered.metadata.as_ref().unwrap()[METADATA_KEY][0]["outcome"],
        "answered"
    );
    drop(service);
    let recovered = EventService::new(db, EventDelivery::in_memory());
    let cancelled = recovered
        .emit(EventRequest::new(
            session,
            context.clone(),
            TurnCancelledData {
                turn_id: turn,
                reason: Some("PRIVATE_REASON".into()),
                usage: None,
            },
        ))
        .await
        .unwrap();
    let waits = &cancelled.metadata.as_ref().unwrap()[METADATA_KEY];
    assert_eq!(waits.as_array().unwrap().len(), 1);
    assert_eq!(waits[0]["tool_call_id"], "a");
    assert_eq!(waits[0]["outcome"], "cancelled");
    assert_eq!(waits[0]["started_at"], json!(asked_at));
    assert!(!waits.to_string().contains("PRIVATE"));
    assert!(!waits.to_string().contains("https://"));
    let late = recovered
        .emit(EventRequest::new(
            session,
            context,
            ToolCompletedData::success(
                "a".into(),
                everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL.into(),
                vec![ContentPart::tool_result_text(&json!({"outcome":"allow"}))],
                None,
            ),
        ))
        .await
        .unwrap();
    assert!(
        late.metadata
            .as_ref()
            .and_then(|m| m.get(METADATA_KEY))
            .is_none()
    );
}

#[tokio::test]
async fn human_wait_postgres_command_recovery_keeps_outcome_time_and_exports_once() {
    use crate::storage::{ClaimWaitingTurnResult, WaitingTurnResolutionPlan};
    use everruns_core::host::observability::{
        InMemorySpanExporter, OtelEventListener, SdkTracerProvider, TraceConventions,
    };
    let db = Arc::new(StorageBackend::test_database());
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let listener = Arc::new(OtelEventListener::with_tracer_provider(
        provider.clone(),
        false,
        TraceConventions::ALL,
    ));
    let service =
        EventService::with_listeners(db.clone(), EventDelivery::in_memory(), vec![listener]);
    let agent = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);
    let session = db
        .create_session(test_session_input(agent))
        .await
        .unwrap()
        .id;
    let turn = TurnId::new();
    let input = MessageId::new();
    let context = EventContext::turn(turn, input);
    service
        .emit(EventRequest::new(
            session,
            context.clone(),
            TurnStartedData {
                turn_id: turn,
                input_message_id: input,
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
                origin: None,
            },
        ))
        .await
        .unwrap();
    let mut ask = EventRequest::new(
        session,
        context,
        ToolCallRequestedData::with_definitions(
            &[
                ToolCall {
                    id: "q".into(),
                    name: "ask_user".into(),
                    arguments: json!({"questions":[],"expires_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339()}),
                },
                ToolCall {
                    id: "client".into(),
                    name: "client_tool".into(),
                    arguments: json!({}),
                },
            ],
            &[],
        ),
    );
    ask.ts = chrono::Utc::now() - chrono::Duration::seconds(30);
    service.emit(ask).await.unwrap();
    db.set_session_status(DEFAULT_ORG_ID, session, "waiting_for_tool_results")
        .await
        .unwrap();
    let ClaimWaitingTurnResult::Claimed(claim) = db
        .claim_tool_result_command(
            DEFAULT_ORG_ID,
            session,
            vec![ToolCompletedData::success(
                "q".into(),
                "".into(),
                vec![ContentPart::tool_result_text(
                    &json!({"status":"declined","answered_by":"user","answers":[]}),
                )],
                None,
            )],
        )
        .await
        .unwrap()
    else {
        panic!("claim")
    };
    assert_eq!(claim.plan.response["resume"], false);
    let original_time = claim.plan.events[0].ts;
    db.abandon_waiting_turn_claim(
        DEFAULT_ORG_ID,
        session,
        claim.resolution_id,
        claim.claim_token,
    )
    .await
    .unwrap();
    drop(service);
    let service = EventService::with_listeners(
        db.clone(),
        EventDelivery::in_memory(),
        vec![Arc::new(OtelEventListener::with_tracer_provider(
            provider.clone(),
            false,
            TraceConventions::ALL,
        ))],
    );
    let ClaimWaitingTurnResult::Claimed(recovered) = db
        .recover_waiting_turn(
            DEFAULT_ORG_ID,
            session,
            WaitingTurnResolutionPlan {
                kind: "timeout".into(),
                events: vec![],
                session_values: vec![],
                response: Value::Null,
            },
        )
        .await
        .unwrap()
    else {
        panic!("recover")
    };
    assert!(recovered.recovered);
    assert_eq!(
        recovered.plan, claim.plan,
        "stored resolution wins over timeout proposal"
    );
    let event = service
        .emit_waiting_turn_resolution(recovered.plan.events[0].clone(), recovered.resolution_id, 0)
        .await
        .unwrap();
    let duplicate = service
        .emit_waiting_turn_resolution(recovered.plan.events[0].clone(), recovered.resolution_id, 0)
        .await
        .unwrap();
    assert_eq!(event.id, duplicate.id);
    assert_eq!(event.ts, original_time);
    assert_eq!(
        event.metadata.as_ref().unwrap()[METADATA_KEY][0]["outcome"],
        "declined"
    );
    assert!(
        db.park_waiting_turn_claim(
            DEFAULT_ORG_ID,
            session,
            recovered.resolution_id,
            recovered.claim_token
        )
        .await
        .unwrap()
    );
    assert_eq!(
        exporter
            .get_finished_spans()
            .unwrap()
            .iter()
            .filter(|span| span.name == "human.wait")
            .count(),
        1
    );
}
