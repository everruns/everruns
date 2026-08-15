//! Public Framework acceptance tests for session history and resumption.

use std::str::FromStr;

use everruns::prelude::*;

fn test_agent() -> Agent {
    Agent::builder()
        .instructions("Keep the transcript in order.")
        .model(Model::simulated("assistant reply"))
        .build()
        .expect("valid agent")
}

#[test]
fn history_cursor_is_portable_without_exposing_host_types() {
    let cursor = HistoryCursor::from_str("eh1.c2Vzc2lvbg.c25hcHNob3Q").expect("valid token");
    let encoded = cursor.to_string();

    assert_eq!(HistoryCursor::from_str(&encoded).unwrap(), cursor);
    assert!(!format!("{cursor:?}").contains(&encoded));
}

#[test]
fn history_errors_keep_session_identity_typed() {
    let session_id = SessionId::new();
    let error = HistoryError::SessionNotFound { session_id };

    assert!(error.to_string().contains(&session_id.to_string()));

    let resume_error = ResumeError::SessionNotFound { session_id };
    assert!(resume_error.to_string().contains(&session_id.to_string()));
}

#[tokio::test]
async fn new_session_has_one_empty_terminal_page_and_fused_end() {
    let session = InMemoryEngine::new().create(test_agent());
    let mut pages = session.history().pages();

    let page = pages.next_page().await.unwrap().expect("first page");
    assert_eq!(page.session_id, session.session_id());
    assert!(page.is_empty());
    assert!(page.next_cursor.is_none());
    assert!(pages.next_page().await.unwrap().is_none());
    assert!(pages.next_page().await.unwrap().is_none());
}

#[tokio::test]
async fn history_is_stably_ordered_and_pages_at_exact_boundaries() {
    let agent = test_agent();
    let session = InMemoryEngine::new().create(agent.clone());
    session.run("first input").await.unwrap();

    let exact = session.history().limit(2).unwrap().page().await.unwrap();
    assert_eq!(exact.len(), 2);
    assert!(exact.next_cursor.is_none());
    assert_eq!(exact.messages[0].role, MessageRole::User);
    assert_eq!(exact.messages[0].text(), "first input");
    assert_eq!(exact.messages[1].role, MessageRole::Agent);
    assert_eq!(exact.messages[1].text(), "assistant reply");

    session.run("second input").await.unwrap();
    let mut pages = session.history().limit(2).unwrap().pages();
    let first = pages.next_page().await.unwrap().unwrap();
    let second = pages.next_page().await.unwrap().unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert!(first.next_cursor.is_some());
    assert!(second.next_cursor.is_none());
    assert_eq!(second.messages[0].text(), "second input");
    assert_eq!(second.messages[1].text(), "assistant reply");
    assert!(pages.next_page().await.unwrap().is_none());
}

#[tokio::test]
async fn cursor_snapshot_excludes_concurrent_appends() {
    let agent = test_agent();
    let session = InMemoryEngine::new().create(agent.clone());
    session.run("one").await.unwrap();
    session.run("two").await.unwrap();

    let first = session.history().limit(1).unwrap().page().await.unwrap();
    let mut cursor = first.next_cursor.clone();
    assert_eq!(first.messages[0].text(), "one");

    session.run("three after snapshot").await.unwrap();

    let mut snapshot_messages = first.messages;
    while let Some(next) = cursor {
        let page = session
            .history()
            .limit(1)
            .unwrap()
            .after(next)
            .unwrap()
            .page()
            .await
            .unwrap();
        cursor = page.next_cursor.clone();
        snapshot_messages.extend(page.messages);
    }
    assert_eq!(snapshot_messages.len(), 4);
    assert!(
        snapshot_messages
            .iter()
            .all(|message| message.text() != "three after snapshot")
    );

    let current = session.history().page().await.unwrap();
    assert_eq!(current.len(), 6);
}

#[tokio::test]
async fn query_rejects_invalid_cross_session_and_out_of_range_inputs() {
    let agent = test_agent();
    let first = InMemoryEngine::new().create(agent.clone());
    first.run("one").await.unwrap();
    let cursor = first
        .history()
        .limit(1)
        .unwrap()
        .page()
        .await
        .unwrap()
        .next_cursor
        .unwrap();

    let second = InMemoryEngine::new().create(agent.clone());
    assert!(matches!(
        second.history().after(cursor),
        Err(HistoryError::CrossSessionCursor)
    ));
    let wrong_version = HistoryCursor::from_str("eh2.dGVzdA").unwrap();
    assert!(matches!(
        first.history().after(wrong_version),
        Err(HistoryError::IncompatibleCursor)
    ));
    let malformed = HistoryCursor::from_str("eh1.bad~").unwrap();
    assert!(matches!(
        first.history().after(malformed),
        Err(HistoryError::InvalidCursor)
    ));
    assert!(matches!(
        first.history().limit(0),
        Err(HistoryError::InvalidLimit {
            requested: 0,
            maximum: 256
        })
    ));
    assert!(matches!(
        first.history().limit(257),
        Err(HistoryError::InvalidLimit {
            requested: 257,
            maximum: 256
        })
    ));
}

#[tokio::test]
async fn engine_resumes_its_sessions_but_a_new_engine_does_not() {
    let engine = InMemoryEngine::new();
    let agent = test_agent();
    let session = engine.create(agent);
    let session_id = session.session_id();
    session.run("remember me").await.unwrap();
    drop(session);

    let resumed = engine.resume(session_id).await.unwrap();
    let page = resumed.history().page().await.unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page.messages[0].text(), "remember me");

    assert!(matches!(
        InMemoryEngine::new().resume(session_id).await,
        Err(ResumeError::SessionNotFound { session_id: missing }) if missing == session_id
    ));
}

#[tokio::test]
async fn engine_can_resume_an_unmaterialized_empty_session() {
    let engine = InMemoryEngine::new();
    let agent = test_agent();
    let session_id = engine.create(agent).session_id();

    let resumed = engine.resume(session_id).await.unwrap();
    assert!(resumed.history().page().await.unwrap().is_empty());
}

#[cfg(feature = "local")]
fn local_agent(data_dir: &std::path::Path) -> Agent {
    Agent::builder()
        .instructions("Keep durable history.")
        .model(Model::simulated("durable reply"))
        .local(LocalConfig::new(data_dir))
        .build()
        .unwrap()
}

#[cfg(feature = "local")]
async fn resume_local(agent: Agent, session_id: SessionId) -> Result<Session, ResumeError> {
    let engine = InMemoryEngine::new();
    engine.attach(session_id, agent).await?;
    engine.resume(session_id).await
}

#[cfg(feature = "local")]
#[tokio::test]
async fn local_profile_resumes_empty_and_multi_turn_sessions_across_agents() {
    let root = tempfile::tempdir().unwrap();
    let session_id = {
        let engine = InMemoryEngine::new();
        let agent = local_agent(root.path());
        let session = engine.create(agent);
        let session_id = session.session_id();
        assert!(session.history().page().await.unwrap().is_empty());
        session_id
    };

    {
        let resumed = resume_local(local_agent(root.path()), session_id)
            .await
            .unwrap();
        assert!(resumed.history().page().await.unwrap().is_empty());
        resumed.run("persisted input").await.unwrap();
    }

    let resumed = resume_local(local_agent(root.path()), session_id)
        .await
        .unwrap();
    let history = resumed.history().page().await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history.messages[0].text(), "persisted input");
    assert_eq!(history.messages[1].text(), "durable reply");
    resumed.run("third turn").await.unwrap();
    assert_eq!(resumed.history().page().await.unwrap().len(), 4);
}

#[cfg(feature = "local")]
#[tokio::test]
async fn repeated_attach_keeps_the_engine_original_agent_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let session_id = {
        let engine = InMemoryEngine::new();
        let session = engine.create(local_agent(root.path()));
        session.start().await.unwrap();
        session.session_id()
    };

    let build_agent = |response| {
        Agent::builder()
            .instructions("Keep durable history.")
            .model(Model::simulated(response))
            .local(LocalConfig::new(root.path()))
            .build()
            .unwrap()
    };
    let engine = InMemoryEngine::new();
    engine
        .attach(session_id, build_agent("first snapshot"))
        .await
        .unwrap();
    engine
        .attach(session_id, build_agent("replacement snapshot"))
        .await
        .unwrap();

    let resumed = engine.resume(session_id).await.unwrap();
    let turn = resumed.run("Which snapshot runs?").await.unwrap();
    assert_eq!(turn.response, "first snapshot");
}

#[cfg(feature = "local")]
#[tokio::test]
async fn local_session_catalog_does_not_serialize_agent_credentials() {
    let root = tempfile::tempdir().unwrap();
    let secret = "catalog-must-not-store-this-secret";
    {
        let agent = Agent::builder()
            .instructions("Keep durable history.")
            .model(Model::simulated("ok"))
            .mcp_server(
                McpServer::http("private", "https://private.example/mcp")
                    .header("Authorization", format!("Bearer {secret}")),
            )
            .local(LocalConfig::new(root.path()))
            .build()
            .unwrap();
        InMemoryEngine::new()
            .create(agent)
            .history()
            .page()
            .await
            .unwrap();
    }

    for entry in std::fs::read_dir(root.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
                "credential sentinel leaked into {}",
                path.display()
            );
        }
    }
}

#[cfg(feature = "local")]
#[tokio::test]
async fn local_resume_distinguishes_missing_corrupt_torn_and_unavailable_state() {
    use std::io::Write;

    let missing_root = tempfile::tempdir().unwrap();
    let missing_id = SessionId::new();
    assert!(matches!(
        resume_local(local_agent(missing_root.path()), missing_id).await,
        Err(ResumeError::SessionNotFound { session_id }) if session_id == missing_id
    ));

    let torn_root = tempfile::tempdir().unwrap();
    let torn_id = {
        let engine = InMemoryEngine::new();
        let agent = local_agent(torn_root.path());
        let session = engine.create(agent);
        session.run("committed").await.unwrap();
        session.session_id()
    };
    let event_path = torn_root.path().join("events.jsonl");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&event_path)
        .unwrap()
        .write_all(b"{\"torn\":")
        .unwrap();
    let recovered = resume_local(local_agent(torn_root.path()), torn_id)
        .await
        .unwrap();
    assert_eq!(recovered.history().page().await.unwrap().len(), 2);

    let corrupt_root = tempfile::tempdir().unwrap();
    let corrupt_id = {
        let engine = InMemoryEngine::new();
        let agent = local_agent(corrupt_root.path());
        let session = engine.create(agent);
        session.run("committed").await.unwrap();
        session.session_id()
    };
    std::fs::OpenOptions::new()
        .append(true)
        .open(corrupt_root.path().join("events.jsonl"))
        .unwrap()
        .write_all(b"not-json\n")
        .unwrap();
    assert!(matches!(
        resume_local(local_agent(corrupt_root.path()), corrupt_id).await,
        Err(ResumeError::Corrupt)
    ));

    let blocked_root = tempfile::tempdir().unwrap();
    let blocked_path = blocked_root.path().join("not-a-directory");
    std::fs::write(&blocked_path, b"file").unwrap();
    let unavailable = Agent::builder()
        .instructions("test")
        .model(Model::simulated("ok"))
        .local(LocalConfig::new(&blocked_path))
        .build()
        .unwrap();
    assert!(matches!(
        resume_local(unavailable, SessionId::new()).await,
        Err(ResumeError::Unavailable)
    ));
}
