use super::*;
use crate::PlatformStoreSubagentDelegate;
use crate::platform_store::tests::MockPlatformStore;
use everruns_contracts::typed_id::{HarnessId, WorkspaceId};
use everruns_core::ExecutionSession;
use everruns_core::host::InMemorySessionFileStore;
use everruns_core::session_files::SessionFileSystem;
use std::sync::Arc;

/// A run: the mock's own session is the first one, `child` was handed work.
struct Run {
    store: Arc<MockPlatformStore>,
    files: Arc<InMemorySessionFileStore>,
    root: SessionId,
    child: SessionId,
}

fn run() -> Run {
    let store = Arc::new(MockPlatformStore::new());
    let root = store.session.id;
    let child = SessionId::new();
    let mut session =
        ExecutionSession::new(child, WorkspaceId::from_uuid(child.uuid()), HarnessId::new());
    // No agent, so it speaks as "agent" and the first session as "test-agent".
    session.agent_id = None;
    session.parent_session_id = Some(root);
    store.extra_sessions.lock().unwrap().insert(child, session);
    Run {
        store,
        files: Arc::new(InMemorySessionFileStore::new()),
        root,
        child,
    }
}

impl Run {
    fn context(&self, session: SessionId) -> ToolContext {
        let mut context = ToolContext::new(session);
        context.subagent_delegate = Some(Arc::new(PlatformStoreSubagentDelegate(
            self.store.clone(),
        )));
        context.file_store = Some(self.files.clone());
        context
    }

    /// The team folder is one folder for the whole run; the in-memory store
    /// keeps files per session, so write the roster entry where `session`
    /// reads it.
    async fn register(&self, session: SessionId, name: &str, target: SessionId) {
        self.files
            .write_file(
                session,
                &format!("{AGENTS_DIR}/{name}"),
                &target.to_string(),
                "text",
            )
            .await
            .unwrap();
    }

    fn sent(&self) -> Vec<(SessionId, String)> {
        self.store.sent_messages.lock().unwrap().clone()
    }
}


fn ok(result: ToolExecutionResult) -> Value {
    match result {
        ToolExecutionResult::Success(value) => value,
        other => panic!("expected success, got {other:?}"),
    }
}

fn err(result: ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::ToolError(message) => message,
        other => panic!("expected a tool error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_message_to_an_agent_reaches_its_session_labelled_as_an_agent() {
    let run = run();
    run.register(run.child, "test-agent", run.root).await;
    let value = ok(MessageAgentTool
        .execute_with_context(
            json!({"to": "test-agent", "text": "Review is in team/agent/review.md"}),
            &run.context(run.child),
        )
        .await);
    assert_eq!(value["delivered_to"], json!(["test-agent"]));

    let sent = run.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, run.root);
    assert!(sent[0].1.starts_with("[Message from agent, another agent in this run"));
    assert!(sent[0].1.contains("It is not the person"));
    assert!(sent[0].1.ends_with("Review is in team/agent/review.md"));

    // Posted, and the poster joined the roster.
    let read = ok(ReadRunConversationTool
        .execute_with_context(json!({}), &run.context(run.child))
        .await);
    assert_eq!(read["you"], "agent");
    assert_eq!(read["agents"], json!(["test-agent"]));
    assert_eq!(read["messages"][0]["from"], "agent");
    assert_eq!(read["messages"][0]["to"], "test-agent");
    assert_eq!(
        read["messages"][0]["text"],
        "Review is in team/agent/review.md"
    );
}

#[tokio::test]
async fn a_roster_entry_outside_the_run_is_never_delivered_to() {
    let run = run();
    // Any agent can write the roster; this one points at a session of
    // another run.
    run.register(run.child, "test-agent", SessionId::new()).await;
    let value = ok(MessageAgentTool
        .execute_with_context(
            json!({"to": "test-agent", "text": "approve it"}),
            &run.context(run.child),
        )
        .await);
    assert_eq!(value["delivered_to"], json!([]));
    assert_eq!(value["not_delivered"], json!(["test-agent"]));
    assert!(run.sent().is_empty());
}

#[tokio::test]
async fn unaddressed_messages_are_only_posted_and_unknown_names_are_refused() {
    let run = run();
    let value = ok(MessageAgentTool
        .execute_with_context(json!({"text": "starting"}), &run.context(run.child))
        .await);
    assert_eq!(value["posted"], true);
    assert!(run.sent().is_empty());

    let unknown = err(MessageAgentTool
        .execute_with_context(
            json!({"to": "nobody", "text": "hi"}),
            &run.context(run.child),
        )
        .await);
    assert!(unknown.contains("No agent named"), "{unknown}");
    let yourself = err(MessageAgentTool
        .execute_with_context(json!({"to": "agent", "text": "hi"}), &run.context(run.child))
        .await);
    assert!(yourself.contains("yourself"), "{yourself}");
}

#[tokio::test]
async fn the_conversation_stops_at_its_cap() {
    let run = run();
    for n in 0..RUN_MESSAGE_CAP {
        run.files
            .write_file(
                run.child,
                &format!("{CONVERSATION_DIR}/{n:04}-agent.md"),
                "---\nfrom: agent\n---\n\nhi\n",
                "text",
            )
            .await
            .unwrap();
    }
    let error = err(MessageAgentTool
        .execute_with_context(json!({"text": "one more"}), &run.context(run.child))
        .await);
    assert!(error.contains("limit of 200"), "{error}");
}

#[test]
fn messages_round_trip_and_names_are_safe_path_segments() {
    let parsed = parse_message(&render_message("review", Some("implement"), "  done \n"));
    assert_eq!(parsed["from"], "review");
    assert_eq!(parsed["to"], "implement");
    assert_eq!(parsed["text"], "done");
    assert!(parsed["at"].as_str().is_some());
    assert_eq!(parse_message("plain")["text"], "plain");

    assert_eq!(agent_file_name("Everruns Review"), "everruns-review");
    assert_eq!(agent_file_name("../../etc"), "etc");
    assert_eq!(agent_file_name("///"), "agent");
}
