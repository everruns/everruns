// LocalPlatformStore: honest subagent core vs explicit Unsupported.

use async_trait::async_trait;
use everruns::local::{LocalPlatformStore, LocalSessionRunner};
use everruns_core::session::ExecutionSession;
use everruns_platform::{PlatformMessage, PlatformStore};
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

fn store() -> LocalPlatformStore {
    LocalPlatformStore::new(Arc::new(FakeRunner::default()), "http://localhost:9300")
}

#[tokio::test]
async fn subagent_core_is_honest() {
    let store = store();
    let harness_id = HarnessId::new();
    let child = store
        .create_session(harness_id, None, Some("child"), None, None, None, None)
        .await
        .unwrap();
    assert_eq!(child.harness_id, harness_id);

    store.send_message(child.id, "go").await.unwrap();
    assert_eq!(store.wait_for_idle(child.id, None).await.unwrap(), "idle");
    assert_eq!(store.get_messages(child.id, None).await.unwrap().len(), 1);
    assert_eq!(store.base_url(), "http://localhost:9300");
}

#[tokio::test]
async fn platform_management_ops_are_unsupported() {
    let store = store();
    assert!(store.list_harnesses().await.is_err());
    assert!(store.list_agents().await.is_err());
    assert!(store.list_apps(None, false).await.is_err());
    assert!(store.list_capabilities(None).await.is_err());
    assert!(store.delete_session(SessionId::new()).await.is_err());
    assert!(
        store
            .create_agent("x", None, None, "prompt", &[])
            .await
            .is_err()
    );
}
