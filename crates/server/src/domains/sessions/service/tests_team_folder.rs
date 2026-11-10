//! Tests: the team folder every agent in one run shares
//! (`knowledge/runtime-resources/agents-using-agents.md`).

use super::*;
use crate::domains::agents::record::Agent;
use crate::domains::agents::types::CreateAgentRequest;
use crate::domains::common::{Command, Ctx};
use crate::kernel_imports::{Caller, DEFAULT_ORG_ID};
use crate::storage::StorageBackend;

use super::tests_support::*;

async fn create_agent(ctx: &Ctx, name: &str, capabilities: Vec<AgentCapabilityConfig>) -> Agent {
    crate::domains::agents::CreateAgent(CreateAgentRequest {
        model_role: None,
        owner_agent_id: None,
        localizations: Default::default(),
        service_virtual_user_id: None,
        id: None,
        name: name.to_string(),
        display_name: Some(name.to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "Agent prompt".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities,
        sandbox_policy: None,
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        max_output_tokens: None,
        communication: Default::default(),
        completion_check: Default::default(),
    })
    .execute(ctx)
    .await
    .unwrap()
}

async fn start(
    service: &SessionService,
    caller: &Caller,
    harness_id: HarnessId,
    agent: &Agent,
    parent: Option<SessionId>,
) -> Session {
    let mut request = build_create_request(harness_id, Some(agent.public_id), None);
    request.parent_session_id = parent;
    service
        .create(
            caller,
            harness_id.uuid(),
            Some(agent.internal_id),
            Some(agent.public_id),
            SessionSource::Api,
            request,
        )
        .await
        .unwrap()
}

fn note(path: &str, content: &str) -> CreateFileInput {
    CreateFileInput {
        path: path.to_string(),
        content: Some(content.to_string()),
        encoding: Some("text".to_string()),
        is_readonly: Some(false),
    }
}

#[tokio::test]
async fn agents_in_one_run_share_the_team_folder() {
    let db = Arc::new(StorageBackend::test_database());
    let service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let harness_id = crate::setup::org_init::generic_harness_id(&db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let lead = create_agent(
        &ctx,
        "lead",
        vec![AgentCapabilityConfig::new("agent_handoff")],
    )
    .await;
    let reviewer = create_agent(&ctx, "reviewer", vec![]).await;

    let first = start(&service, &caller, harness_id, &lead, None).await;
    let review = start(&service, &caller, harness_id, &reviewer, Some(first.id)).await;

    let files = WorkspaceFileService::new(db.clone());
    files
        .create_file(
            review.id.uuid(),
            note("/team/reviewer/findings.md", "ship it"),
        )
        .await
        .unwrap();

    let read = files
        .read_file(first.id.uuid(), "/team/reviewer/findings.md")
        .await
        .unwrap()
        .expect("the first session reads what the reviewer wrote");
    assert_eq!(read.content.as_deref(), Some("ship it"));
    assert!(
        db.get_session_file(review.id.uuid(), "/team/reviewer/findings.md")
            .await
            .unwrap()
            .is_none(),
        "team folder writes outlive the session"
    );

    // A second run of the same lead gets a folder of its own.
    let next = start(&service, &caller, harness_id, &lead, None).await;
    assert!(
        files
            .read_file(next.id.uuid(), "/team/reviewer/findings.md")
            .await
            .unwrap()
            .is_none()
    );

    // The folder is the run's, not the organization's memory.
    assert!(
        db.list_memories(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_session_outside_any_team_has_no_team_folder() {
    let db = Arc::new(StorageBackend::test_database());
    let service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let harness_id = crate::setup::org_init::generic_harness_id(&db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let solo = create_agent(&ctx, "solo", vec![]).await;
    let session = start(&service, &caller, harness_id, &solo, None).await;

    let files = WorkspaceFileService::new(db.clone());
    assert!(
        files
            .stat(session.id.uuid(), "/team")
            .await
            .unwrap()
            .is_none()
    );
    // Writing there lands in the session's own files, not in a shared folder.
    files
        .create_file(session.id.uuid(), note("/team/notes.md", "mine"))
        .await
        .unwrap();
    assert!(
        db.get_session_file(session.id.uuid(), "/team/notes.md")
            .await
            .unwrap()
            .is_some()
    );
}
