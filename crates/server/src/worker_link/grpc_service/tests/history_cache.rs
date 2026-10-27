//! A worker's cached session history matches a fresh full read as the session
//! grows, including the latest-N window (`history_cache` in the worker).

use super::*;
use everruns_contracts::typed_id::SessionId;
use everruns_core::MessageRetriever;
use everruns_core::events::{InputMessageData, OutputMessageCompletedData};
use everruns_core::{EventContext, EventRequest, MessageQuery, RuntimeMessage};
use everruns_worker::grpc_adapters::{GrpcAdapter, GrpcClient};

async fn emit(events: &EventService, session_id: SessionId, message: RuntimeMessage) {
    let request = if message.role == everruns_core::RuntimeMessageRole::User {
        EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(message),
        )
    } else {
        EventRequest::new(
            session_id,
            EventContext::empty(),
            OutputMessageCompletedData::new(message),
        )
    };
    events.emit(request).await.unwrap();
}

fn texts(messages: &[RuntimeMessage]) -> Vec<String> {
    messages.iter().map(|m| m.content_to_llm_string()).collect()
}

#[tokio::test]
async fn cached_history_matches_a_fresh_read_as_the_session_grows() {
    let service = test_worker_service().await;
    let (session_id, _) = create_grpc_test_session(&service).await;
    let events = service.event_service.clone();
    emit(&events, session_id, RuntimeMessage::user("one")).await;
    let (addr, shutdown, server) = start_grpc_test_server(service).await;

    // One long-lived worker (the cache) against a new client per read.
    let cached = GrpcAdapter::new(GrpcClient::connect(&addr).await.unwrap());
    async fn fresh(addr: &str, session_id: SessionId) -> Vec<RuntimeMessage> {
        let adapter = GrpcAdapter::new(GrpcClient::connect(addr).await.unwrap());
        adapter.load(session_id).await.unwrap()
    }

    assert_eq!(texts(&cached.load(session_id).await.unwrap()), ["one"]);

    for message in [
        RuntimeMessage::assistant("two"),
        RuntimeMessage::user("three"),
        RuntimeMessage::assistant("four"),
    ] {
        emit(&events, session_id, message).await;
        let from_cache = cached.load(session_id).await.unwrap();
        assert_eq!(texts(&from_cache), texts(&fresh(&addr, session_id).await));
    }
    assert_eq!(
        texts(&cached.load(session_id).await.unwrap()),
        ["one", "two", "three", "four"]
    );

    // The latest-N window is cut from the cached history.
    let window = cached
        .load_filtered_history(MessageQuery::new(session_id).with_limit(2))
        .await
        .unwrap();
    assert_eq!(texts(&window.messages), ["three", "four"]);
    let fresh_window = GrpcAdapter::new(GrpcClient::connect(&addr).await.unwrap())
        .load_filtered_history(MessageQuery::new(session_id).with_limit(2))
        .await
        .unwrap();
    assert_eq!(texts(&fresh_window.messages), ["three", "four"]);
    assert_eq!(window.source_sequence, fresh_window.source_sequence);

    let _ = shutdown.send(());
    let _ = server.await;
}

/// A cold latest-N read is answered by the server's window, and a later full
/// read still returns the whole history.
#[tokio::test]
async fn cold_window_read_returns_the_server_window() {
    let service = test_worker_service().await;
    let (session_id, _) = create_grpc_test_session(&service).await;
    let events = service.event_service.clone();
    for text in ["a", "b", "c"] {
        emit(&events, session_id, RuntimeMessage::user(text)).await;
    }
    let (addr, shutdown, server) = start_grpc_test_server(service).await;
    let adapter = GrpcAdapter::new(GrpcClient::connect(&addr).await.unwrap());

    let window = adapter
        .load_filtered_history(MessageQuery::new(session_id).with_limit(1))
        .await
        .unwrap();
    assert_eq!(texts(&window.messages), ["c"]);
    assert_eq!(
        texts(&adapter.load(session_id).await.unwrap()),
        ["a", "b", "c"]
    );

    let _ = shutdown.send(());
    let _ = server.await;
}
