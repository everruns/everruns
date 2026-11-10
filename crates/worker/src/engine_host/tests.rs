//! The worker's Engine ports, without a control plane.

use crate::core::events::{EventContext, EventData, EventRequest, ReasonStartedData};
use everruns::{WorkspaceBackend, WorkspaceError, WorkspaceHeadAccess, WorkspaceHeadRequest};
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};

use super::backends::EventStamp;
use super::files::{PlatformFilesBackend, PlatformWorkspace};
use crate::unified_worker_test_adapters::NoopAdapters;

fn reason_started(agent: Option<AgentId>, harness: HarnessId) -> EventRequest {
    EventRequest::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::ReasonStarted(ReasonStartedData {
            harness_id: harness,
            agent_id: agent,
            metadata: None,
        }),
    )
}

/// The events the Engine writes name the platform's agent and harness, and
/// carry the agent tag a worker turn's events carry.
#[test]
fn the_event_port_names_the_platform_agent() {
    let (engine_agent, engine_harness) = (AgentId::new(), HarnessId::new());
    let (agent, harness) = (AgentId::new(), HarnessId::new());
    let stamp = EventStamp {
        agent_id: Some(agent),
        harness_id: Some(harness),
    };
    let mut request = reason_started(Some(engine_agent), engine_harness);
    stamp.apply(&mut request);
    let EventData::ReasonStarted(data) = &request.data else {
        panic!("still reason.started");
    };
    assert_eq!(data.agent_id, Some(agent));
    assert_eq!(data.harness_id, harness);
    assert_eq!(
        request
            .metadata
            .as_ref()
            .and_then(|m| m["agent_id"].as_str()),
        Some(agent.to_string().as_str())
    );

    // A harness-only session names no agent; nothing is tagged.
    let mut request = reason_started(Some(engine_agent), engine_harness);
    EventStamp::default().apply(&mut request);
    let EventData::ReasonStarted(data) = &request.data else {
        panic!("still reason.started");
    };
    assert_eq!(data.agent_id, None);
    assert_eq!(data.harness_id, engine_harness);
    assert!(request.metadata.is_none());
}

/// A session's head is named by the session, in the platform workspace its
/// files live in (whose id is the key the file tools scope by), and any
/// worker reopens it from the binding alone.
#[tokio::test]
async fn session_files_reopen_from_the_binding_alone() {
    let backend = PlatformFilesBackend::new(NoopAdapters);
    let session = SessionId::new();
    let platform = PlatformWorkspace {
        org: 42,
        workspace: session.uuid(),
    };
    let workspace = backend.open_workspace(&platform.locator()).await.unwrap();
    assert_eq!(workspace.id.uuid(), session.uuid());
    let head = backend
        .create_head(
            &workspace,
            WorkspaceHeadRequest {
                name: session.uuid().to_string(),
                base: None,
                access: WorkspaceHeadAccess::Isolated,
            },
        )
        .await
        .unwrap();
    assert_eq!(head.head.id.uuid(), session.uuid());
    assert_eq!(head.binding.payload, platform.locator().into_bytes());

    let other = PlatformFilesBackend::new(NoopAdapters);
    let reopened = other.reopen_head(&head.binding).await.unwrap();
    assert_eq!(reopened.workspace.id, workspace.id);

    // A binding whose payload names another workspace is refused.
    let mut foreign = head.binding.clone();
    foreign.payload = PlatformWorkspace {
        org: 43,
        workspace: uuid::Uuid::now_v7(),
    }
    .locator()
    .into_bytes();
    assert!(matches!(
        other.reopen_head(&foreign).await,
        Err(WorkspaceError::BindingMismatch)
    ));
    assert!(matches!(
        backend.open_workspace("42").await,
        Err(WorkspaceError::NotFound)
    ));
}
