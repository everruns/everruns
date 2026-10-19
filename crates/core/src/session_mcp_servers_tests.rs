use super::*;
use crate::ard_attachment::{
    ArdAttachment, ArdAttachmentTarget, apply_session_attachments, attachment_kv_key,
};
use crate::mcp_server::McpServerTransportType;
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::session_services::{KeyInfo, SecretInfo};
use std::collections::HashMap;
use std::sync::Mutex;

/// Session-keyed KV store, so tests can tell one session's records from another's.
#[derive(Default)]
struct MemoryStorage(Mutex<HashMap<(SessionId, String), String>>);

#[async_trait]
impl SessionStorageStore for MemoryStorage {
    async fn set_value(&self, session: SessionId, key: &str, value: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert((session, key.to_string()), value.to_string());
        Ok(())
    }
    async fn get_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(session, key.to_string()))
            .cloned())
    }
    async fn delete_value(&self, session: SessionId, key: &str) -> Result<bool> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .remove(&(session, key.to_string()))
            .is_some())
    }
    async fn list_keys(&self, session: SessionId) -> Result<Vec<KeyInfo>> {
        let now = chrono::Utc::now();
        Ok(self
            .0
            .lock()
            .unwrap()
            .keys()
            .filter(|(s, _)| *s == session)
            .map(|(_, key)| KeyInfo {
                key: key.clone(),
                created_at: now,
                updated_at: now,
            })
            .collect())
    }
    async fn set_secret(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    async fn get_secret(&self, _: SessionId, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn delete_secret(&self, _: SessionId, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn list_secrets(&self, _: SessionId) -> Result<Vec<SecretInfo>> {
        Ok(Vec::new())
    }
}

fn session() -> ExecutionSession {
    ExecutionSession::with_own_workspace(SessionId::new(), crate::typed_id::HarnessId::new())
}

fn http(url: &str) -> ScopedMcpServer {
    ScopedMcpServer {
        transport_type: McpServerTransportType::Http,
        url: url.to_string(),
        ..Default::default()
    }
}

fn chat_server(name: &str, url: &str) -> SessionMcpServer {
    SessionMcpServer {
        name: name.to_string(),
        server: http(url),
        source: SessionMcpServerSource::UserMcp,
    }
}

fn ard_mcp(urn: &str, name: &str) -> ArdAttachment {
    ArdAttachment {
        urn: urn.to_string(),
        display_name: "Docs".to_string(),
        media_type: "application/mcp-server+json".to_string(),
        registry_id: "public".to_string(),
        target: ArdAttachmentTarget::McpServer {
            name: name.to_string(),
            server: http("https://docs.example.com/mcp"),
        },
    }
}

#[test]
fn record_round_trips_with_its_source() {
    let record = SessionMcpServer {
        name: "docs".into(),
        server: http("https://docs.example.com/mcp"),
        source: SessionMcpServerSource::Ard {
            urn: "urn:ai:x:y:z".into(),
        },
    };
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["source"]["type"], "ard");
    assert_eq!(json["source"]["urn"], "urn:ai:x:y:z");
    let back: SessionMcpServer = serde_json::from_value(json).unwrap();
    assert_eq!(back, record);
    assert_eq!(session_mcp_server_kv_key("docs"), "session_mcp:docs");
}

#[tokio::test]
async fn a_record_joins_the_next_turn_and_removing_it_drops_it() {
    let storage = MemoryStorage::default();
    let mut turn = session();
    let id = turn.id;
    put_session_mcp_server(&storage, id, &chat_server("notes", "https://n.example/mcp"))
        .await
        .unwrap();

    apply_session_attachments(&storage, &mut turn).await;
    assert_eq!(turn.mcp_servers["notes"].url, "https://n.example/mcp");
    assert_eq!(
        get_session_mcp_server(&storage, id, "notes")
            .await
            .unwrap()
            .source,
        SessionMcpServerSource::UserMcp
    );

    assert!(
        remove_session_mcp_server(&storage, id, "notes")
            .await
            .unwrap()
    );
    assert!(
        !remove_session_mcp_server(&storage, id, "notes")
            .await
            .unwrap()
    );
    let mut next = session();
    next.id = id;
    apply_session_attachments(&storage, &mut next).await;
    assert!(next.mcp_servers.is_empty());
}

#[tokio::test]
async fn records_stay_in_their_own_session() {
    let storage = MemoryStorage::default();
    let mut mine = session();
    let mut other = session();
    put_session_mcp_server(
        &storage,
        mine.id,
        &chat_server("notes", "https://n.example/mcp"),
    )
    .await
    .unwrap();

    apply_session_attachments(&storage, &mut other).await;
    assert!(other.mcp_servers.is_empty());
    apply_session_attachments(&storage, &mut mine).await;
    assert!(mine.mcp_servers.contains_key("notes"));
}

#[tokio::test]
async fn same_name_keeps_one_record_last_write_wins() {
    let storage = MemoryStorage::default();
    let mut turn = session();
    let record = ard_mcp("urn:ai:x:y:z", "docs")
        .session_mcp_server()
        .unwrap();
    put_session_mcp_server(&storage, turn.id, &record)
        .await
        .unwrap();
    put_session_mcp_server(
        &storage,
        turn.id,
        &chat_server("docs", "https://mine.example/mcp"),
    )
    .await
    .unwrap();

    let records = load_session_mcp_servers(&storage, turn.id).await;
    assert_eq!(records.len(), 1);
    apply_session_attachments(&storage, &mut turn).await;
    assert_eq!(turn.mcp_servers["docs"].url, "https://mine.example/mcp");
}

#[tokio::test]
async fn a_record_wins_over_the_session_configured_server() {
    let storage = MemoryStorage::default();
    let mut turn = session();
    turn.mcp_servers
        .insert("docs".into(), http("https://configured.example/mcp"));
    put_session_mcp_server(
        &storage,
        turn.id,
        &chat_server("docs", "https://chat.example/mcp"),
    )
    .await
    .unwrap();
    apply_session_attachments(&storage, &mut turn).await;
    assert_eq!(turn.mcp_servers["docs"].url, "https://chat.example/mcp");
}

#[tokio::test]
async fn an_ard_mcp_target_joins_only_through_its_session_record() {
    let storage = MemoryStorage::default();
    let mut turn = session();
    let attachment = ard_mcp("urn:ai:x:y:z", "docs");
    storage
        .set_value(
            turn.id,
            &attachment_kv_key(&attachment.slug()),
            &serde_json::to_string(&attachment).unwrap(),
        )
        .await
        .unwrap();

    apply_session_attachments(&storage, &mut turn).await;
    assert!(
        turn.mcp_servers.is_empty(),
        "the ard_attach record alone adds no server"
    );

    let record = attachment.session_mcp_server().unwrap();
    assert_eq!(
        record.source,
        SessionMcpServerSource::Ard {
            urn: "urn:ai:x:y:z".into()
        }
    );
    put_session_mcp_server(&storage, turn.id, &record)
        .await
        .unwrap();
    apply_session_attachments(&storage, &mut turn).await;
    assert_eq!(turn.mcp_servers["docs"].url, "https://docs.example.com/mcp");
}

#[tokio::test]
async fn a_malformed_record_is_skipped() {
    let storage = MemoryStorage::default();
    let mut turn = session();
    storage
        .set_value(turn.id, "session_mcp:broken", "not json")
        .await
        .unwrap();
    put_session_mcp_server(
        &storage,
        turn.id,
        &chat_server("ok", "https://ok.example/mcp"),
    )
    .await
    .unwrap();
    apply_session_attachments(&storage, &mut turn).await;
    assert_eq!(turn.mcp_servers.len(), 1);
    assert!(turn.mcp_servers.contains_key("ok"));
}
