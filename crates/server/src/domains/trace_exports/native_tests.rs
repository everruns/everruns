use super::{test_support::*, *};
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::{EgressScope, Event, EventContext};
use serde_json::json;
use std::sync::Arc;

#[test]
fn native_destinations_validate_endpoint_project_and_credentials() {
    for provider in [TraceProvider::Langfuse, TraceProvider::Langsmith] {
        let mut c = config();
        c.provider = provider.clone();
        assert!(c.validate().is_err());
        c.endpoint = "https://example.com/otel/v1/traces".into();
        c.project_name = Some("production".into());
        assert!(c.validate().is_ok());
        let empty = TraceSecrets {
            headers: Default::default(),
        };
        assert!(empty.validate_for(&provider).is_err());
        c.project_name = Some("bad\r\nHost: evil".into());
        assert!(c.validate().is_err());
    }
    let langfuse = TraceSecrets {
        headers: [("Authorization".into(), "Basic cGs6c2s=".into())].into(),
    };
    assert!(langfuse.validate_for(&TraceProvider::Langfuse).is_ok());
    let missing_secret = TraceSecrets {
        headers: [("Authorization".into(), "Basic cGs6".into())].into(),
    };
    assert!(
        missing_secret
            .validate_for(&TraceProvider::Langfuse)
            .is_err()
    );
    let langsmith = TraceSecrets {
        headers: [("x-api-key".into(), "test-key".into())].into(),
    };
    assert!(langsmith.validate_for(&TraceProvider::Langsmith).is_ok());
}

#[test]
fn langsmith_agent_routing_rejects_incomplete_conflicting_and_invalid_settings() {
    let mut c = config();
    c.provider = TraceProvider::Langsmith;
    c.endpoint = "https://api.smith.langchain.com/otel/v1/traces".into();
    c.langsmith_agent_id = Some("my-agent-1".into());
    assert!(c.validate().is_err());
    c.langsmith_agent_environment = Some("production".into());
    assert!(c.validate().is_ok());
    c.project_name = Some("conflicting-project".into());
    assert!(c.validate().is_err());
    c.project_name = None;
    for id in ["", "Uppercase", "1first", "trailing-", "bad\r\nHeader"] {
        c.langsmith_agent_id = Some(id.into());
        assert!(c.validate().is_err(), "accepted {id:?}");
    }
    for id in ["a".to_string(), "a".repeat(63)] {
        c.langsmith_agent_id = Some(id);
        assert!(c.validate().is_ok());
    }
    c.langsmith_agent_id = Some("a".repeat(64));
    assert!(c.validate().is_err());
    c.langsmith_agent_id = Some("my-agent".into());
    c.provider = TraceProvider::Otlp;
    assert!(c.validate().is_err());
    c.provider = TraceProvider::Langsmith;
    c.langsmith_agent_environment = Some("arbitrary".into());
    assert!(c.validate().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_destinations_use_scoped_auth_vendor_mapping_and_privacy() {
    for (provider, capture, agent_workspace) in [
        (TraceProvider::Langfuse, false, false),
        (TraceProvider::Langsmith, false, false),
        (TraceProvider::Langfuse, true, false),
        (TraceProvider::Langsmith, true, false),
        (TraceProvider::Langsmith, false, true),
    ] {
        let db = Arc::new(StorageBackend::test_database());
        let mut c = config();
        c.provider = provider.clone();
        c.capture_content = capture;
        c.endpoint = "https://collector.example/otel/v1/traces".into();
        if agent_workspace {
            c.langsmith_agent_id = Some("native-agent".into());
            c.langsmith_agent_environment = Some("staging".into());
        } else {
            c.project_name = Some("selected-project".into());
        }
        let credentials = match provider {
            TraceProvider::Langfuse => json!({"headers":{"Authorization":"Basic cGs6c2s="}}),
            _ => json!({"headers":{"x-api-key":"SCOPED_KEY"}}),
        };
        let id = db
            .create_tracing_destination(
                1,
                &c,
                encryption()
                    .encrypt_string(&credentials.to_string())
                    .unwrap(),
            )
            .await
            .unwrap();
        let collector = Arc::new(Collector::default());
        let listener = router::make_listener(
            db.clone(),
            encryption(),
            collector.clone(),
            db.tracing_destination(1, id).await.unwrap().unwrap(),
            EgressScope::default(),
        )
        .await
        .unwrap();
        let session = SessionId::new();
        let turn = TurnId::new();
        let message = MessageId::new();
        let context = EventContext::turn(turn, message);
        listener
            .on_event(&Event::new(
                session,
                context.clone(),
                everruns_core::events::TurnStartedData {
                    turn_id: turn,
                    input_message_id: message,
                    input_content: Some("PRIVATE_INPUT".into()),
                    agent_id: None,
                    agent_name: Some("Native provider test".into()),
                    agent_description: None,
                    origin: None,
                },
            ))
            .await;
        listener
            .on_event(&Event::new(
                session,
                context,
                everruns_core::events::TurnCompletedData {
                    turn_id: turn,
                    iterations: 1,
                    final_answer_preview: Some("PRIVATE_OUTPUT".into()),
                    ..Default::default()
                },
            ))
            .await;
        listener.flush().await.unwrap();
        let requests = collector.requests.lock().unwrap();
        assert!(!requests.is_empty());
        for request in requests.iter() {
            assert_eq!(request.url, c.endpoint);
            assert!(request.dns_pinning_required);
            let body = String::from_utf8_lossy(&request.body);
            assert_eq!(body.contains("PRIVATE_INPUT"), capture);
            assert_eq!(body.contains("PRIVATE_OUTPUT"), capture);
            if provider == TraceProvider::Langfuse {
                assert_eq!(
                    request.headers.get("x-langfuse-ingestion-version").unwrap(),
                    "4"
                );
                assert_eq!(
                    request.headers.get("Authorization").unwrap(),
                    "Basic cGs6c2s="
                );
                assert!(body.contains("langfuse.observation.type"));
            } else {
                if agent_workspace {
                    assert_eq!(
                        request.headers.get("Langsmith-Agent-Id").unwrap(),
                        "native-agent"
                    );
                    assert_eq!(
                        request.headers.get("Langsmith-Agent-Environment").unwrap(),
                        "staging"
                    );
                    assert!(!request.headers.contains_key("Langsmith-Project"));
                } else {
                    assert_eq!(
                        request.headers.get("Langsmith-Project").unwrap(),
                        "selected-project"
                    );
                    assert!(!request.headers.contains_key("Langsmith-Agent-Id"));
                    assert!(!request.headers.contains_key("Langsmith-Agent-Environment"));
                }
                assert_eq!(request.headers.get("x-api-key").unwrap(), "SCOPED_KEY");
                assert!(body.contains("langsmith.span.kind"));
            }
        }
    }
}

#[tokio::test]
async fn native_otlp_partial_rejection_fails_transport_and_records_delivery_health() {
    use async_trait::async_trait;
    use everruns_core::{
        EgressRequest, EgressRequestKind, EgressResponse, EgressResult, EgressService,
        EgressStreamResponse,
    };
    struct PartialReceiver;
    #[async_trait]
    impl EgressService for PartialReceiver {
        async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
            Ok(EgressResponse {
                status: 200,
                headers: Default::default(),
                body: vec![10, 2, 8, 1],
            })
        }
        async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
            unreachable!()
        }
    }
    let db = Arc::new(StorageBackend::test_database());
    let mut c = config();
    c.provider = TraceProvider::Langfuse;
    c.endpoint = "https://collector.example/otel/v1/traces".into();
    let id = db
        .create_tracing_destination(
            1,
            &c,
            encryption().encrypt_string("{\"headers\":{}}").unwrap(),
        )
        .await
        .unwrap();
    let transport = transport::DestinationTransport {
        db: db.clone(),
        inner: Arc::new(PartialReceiver),
        row: db.tracing_destination(1, id).await.unwrap().unwrap(),
        scope: EgressScope::default(),
        secrets: TraceSecrets {
            headers: Default::default(),
        },
    };
    let result = transport
        .send(EgressRequest::new(
            "POST",
            &c.endpoint,
            EgressRequestKind::Other("tracing".into()),
        ))
        .await;
    assert!(result.is_err());
    let current = db.tracing_destination(1, id).await.unwrap().unwrap();
    assert!(current.last_error.is_some());
    assert!(current.last_accepted_at.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn langsmith_destinations_never_mix_request_routing_or_credentials() {
    let db = Arc::new(StorageBackend::test_database());
    let collector = Arc::new(Collector::default());
    let mut listeners = Vec::new();
    for project in ["project-one", "project-two"] {
        let mut c = config();
        c.provider = TraceProvider::Langsmith;
        c.project_name = Some(project.into());
        c.endpoint = "https://collector.example/otel/v1/traces".into();
        let secret = json!({"headers":{"x-api-key":format!("key-{project}")}});
        let id = db
            .create_tracing_destination(
                1,
                &c,
                encryption().encrypt_string(&secret.to_string()).unwrap(),
            )
            .await
            .unwrap();
        listeners.push(
            router::make_listener(
                db.clone(),
                encryption(),
                collector.clone(),
                db.tracing_destination(1, id).await.unwrap().unwrap(),
                EgressScope::default(),
            )
            .await
            .unwrap(),
        );
    }
    let session = SessionId::new();
    let turn = TurnId::new();
    let message = MessageId::new();
    let context = EventContext::turn(turn, message);
    let started = Event::new(
        session,
        context.clone(),
        everruns_core::events::TurnStartedData {
            turn_id: turn,
            input_message_id: message,
            input_content: None,
            agent_id: None,
            agent_name: Some("Shared turn".into()),
            agent_description: None,
            origin: None,
        },
    );
    let completed = Event::new(
        session,
        context,
        everruns_core::events::TurnCompletedData {
            turn_id: turn,
            iterations: 1,
            ..Default::default()
        },
    );
    for listener in &listeners {
        listener.on_event(&started).await;
        listener.on_event(&completed).await;
    }
    for listener in listeners {
        listener.flush().await.unwrap();
    }
    let requests = collector.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let projects: std::collections::BTreeSet<_> = requests
        .iter()
        .map(|request| {
            let project = request.headers.get("Langsmith-Project").unwrap();
            assert_eq!(
                request.headers.get("x-api-key").unwrap(),
                &format!("key-{project}")
            );
            assert!(!request.headers.contains_key("Langsmith-Agent-Id"));
            assert!(!request.headers.contains_key("Langsmith-Agent-Environment"));
            project.as_str()
        })
        .collect();
    assert_eq!(
        projects,
        ["project-one", "project-two"].into_iter().collect()
    );
}
