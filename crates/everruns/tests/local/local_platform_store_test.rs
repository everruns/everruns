// LocalPlatformStore: honest subagent core vs explicit Unsupported.

use async_trait::async_trait;
use everruns::local::{LocalPlatformStore, LocalSessionRunner};
use everruns_core::session::{ExecutionSession, SessionSeedMode};
use everruns_platform::{PlatformCreateSessionRequest, PlatformMessage, PlatformStore};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Default)]
struct FakeRunner {
    sent: Mutex<Vec<(SessionId, String)>>,
}

#[async_trait]
impl LocalSessionRunner for FakeRunner {
    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        _locale: Option<&str>,
        parent_session_id: Option<SessionId>,
    ) -> Result<ExecutionSession> {
        let id = SessionId::new();
        let mut s = everruns_host::SessionBuilder::new(harness_id)
            .id(id)
            .title(title.unwrap_or("child"))
            .build();
        s.agent_id = agent_id;
        s.parent_session_id = parent_session_id;
        Ok(s)
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        self.sent
            .lock()
            .unwrap()
            .push((session_id, content.to_string()));
        Ok(())
    }

    async fn list_sessions(
        &self,
        _limit: Option<usize>,
        _agent_id: Option<AgentId>,
    ) -> Result<Vec<ExecutionSession>> {
        Ok(vec![])
    }

    async fn get_session(&self, _session_id: SessionId) -> Result<Option<ExecutionSession>> {
        Ok(None)
    }

    async fn get_messages(
        &self,
        _session_id: SessionId,
        _limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        Ok(vec![PlatformMessage {
            role: "agent".into(),
            content: "hi".into(),
            created_at: chrono::Utc::now(),
        }])
    }

    async fn get_session_status(&self, _session_id: SessionId) -> Result<Option<String>> {
        Ok(Some("idle".to_string()))
    }
}

fn request(harness_id: HarnessId, blueprint_id: Option<&str>) -> PlatformCreateSessionRequest {
    PlatformCreateSessionRequest {
        harness_id,
        agent_id: None,
        title: Some("child".to_string()),
        goal: None,
        locale: None,
        blueprint_id: blueprint_id.map(str::to_string),
        blueprint_config: None,
        parent_session_id: None,
        forked_from_session_id: None,
        budget_root_session_id: None,
        seed: SessionSeedMode::Fresh,
    }
}

fn store() -> LocalPlatformStore {
    LocalPlatformStore::new(Arc::new(FakeRunner::default()))
}

#[tokio::test]
async fn subagent_core_is_honest() {
    let store = store();
    let harness_id = HarnessId::new();
    let child = store
        .create_session_with_options(request(harness_id, None))
        .await
        .unwrap();
    assert_eq!(child.harness_id, harness_id);

    store.send_message(child.id, "go").await.unwrap();
    assert_eq!(store.wait_for_idle(child.id, None).await.unwrap(), "idle");
    assert_eq!(store.get_messages(child.id, None).await.unwrap().len(), 1);
}

/// The management surface the local store used to reject method-by-method is
/// gone from `PlatformStore` entirely (EVE-953), so there is nothing left to
/// call. What still has to hold is that the one creation path the local store
/// does not support is refused rather than silently ignored.
#[tokio::test]
async fn blueprint_sessions_are_unsupported() {
    let store = store();
    assert!(
        store
            .create_session_with_options(request(HarnessId::new(), Some("bp")))
            .await
            .is_err()
    );
}
