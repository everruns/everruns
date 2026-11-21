use std::sync::{Arc, Weak};

use everruns_capabilities::capabilities::UserMcpCallInvoker;
use everruns_contracts::dataplane::Connection;
use everruns_contracts::host_calls::{HostCallAnswer, OPEN_TURN, USER_MCP_STORE};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::mcp::{UserMcpStoreCall, UserMcpStoreError, UserMcpStoreReply};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::mcp::PerTurnGrant;
use crate::providers::OrgMaterial;

/// A control plane whose store answers `answer`; every call is recorded.
struct ControlPlane {
    answer: Value,
    asked: Mutex<Vec<Value>>,
}

#[async_trait::async_trait]
impl OrgMaterial for ControlPlane {
    fn tenant(&self, _session: SessionId) -> Option<String> {
        Some("org-a".into())
    }

    async fn connection(&self, _org: &str, _id: &str) -> anyhow::Result<Option<Connection>> {
        Ok(None)
    }

    async fn secret(&self, _org: &str, _secret_id: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("no secrets")
    }

    fn looks_up(&self) -> bool {
        true
    }

    fn declared_host_calls(
        &self,
        _session: SessionId,
    ) -> Option<std::collections::BTreeSet<&'static str>> {
        Some([USER_MCP_STORE].into())
    }

    async fn lookup(
        &self,
        _org: &str,
        command: &str,
        params: Value,
    ) -> anyhow::Result<HostCallAnswer> {
        assert_eq!(command, USER_MCP_STORE);
        self.asked.lock().push(params);
        Ok(HostCallAnswer::Value(self.answer.clone()))
    }
}

fn store(cp: &Arc<ControlPlane>, session: SessionId) -> super::CellUserMcpStore {
    let material: Arc<dyn OrgMaterial> = cp.clone();
    let weak: Weak<dyn OrgMaterial> = Arc::downgrade(&material);
    // Kept alive by the test's `cp`.
    std::mem::forget(material);
    super::CellUserMcpStore {
        lookups: crate::lookups::org_lookups(Some(&weak), session).unwrap(),
        session,
    }
}

#[tokio::test]
async fn the_manage_tools_reach_the_persons_list_for_the_open_turn() {
    let cp = Arc::new(ControlPlane {
        answer: json!({ "ok": { "reply": "removed", "removed": true } }),
        asked: Mutex::default(),
    });
    let session = SessionId::new();
    let reply = store(&cp, session)
        .invoke(
            None,
            UserMcpStoreCall::Remove {
                name: "notes".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(reply, UserMcpStoreReply::Removed { removed: true });
    let asked = cp.asked.lock()[0].clone();
    assert_eq!(asked["session_id"], json!(session.to_string()));
    // The session's cell names the turn and its person, not the executor.
    assert_eq!(asked["input_message_id"], json!(OPEN_TURN));
    assert_eq!(asked["call"]["op"], json!("remove"));
    assert_eq!(asked["call"]["name"], json!("notes"));
}

#[tokio::test]
async fn a_refusal_keeps_its_kind() {
    let cp = Arc::new(ControlPlane {
        answer: json!({ "error": { "kind": "unavailable", "message": "not yours" } }),
        asked: Mutex::default(),
    });
    let error = store(&cp, SessionId::new())
        .invoke(None, UserMcpStoreCall::List)
        .await
        .unwrap_err();
    assert_eq!(error, UserMcpStoreError::Unavailable("not yours".into()));
}

#[test]
fn a_persons_grant_serves_only_their_server_at_its_address() {
    let session = SessionId::new();
    let grant = PerTurnGrant {
        provider: "mcp_oauth_notes".into(),
        acts_as: everruns_core::McpServerActsAs::User,
    };
    super::TURNS.lock().insert(
        session,
        super::Turn {
            message: MessageId::new(),
            servers: Default::default(),
            grants: [(
                "notes".to_owned(),
                ("https://notes.example/mcp".to_owned(), grant.clone()),
            )]
            .into(),
        },
    );
    let url = Some("https://notes.example/mcp");
    assert_eq!(super::grant(session.uuid(), "notes", url), Some(grant));
    // An agent's own server of the same name elsewhere: not the person's.
    assert_eq!(
        super::grant(session.uuid(), "notes", Some("https://other.example/mcp")),
        None
    );
    assert_eq!(super::grant(session.uuid(), "docs", url), None);
    assert_eq!(super::grant(SessionId::new().uuid(), "notes", url), None);
}
