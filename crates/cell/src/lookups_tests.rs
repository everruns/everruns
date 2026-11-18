use std::sync::Arc;

use everruns_contracts::dataplane::{Connection, LookupAnswer};
use everruns_contracts::session_sandbox::SessionSandboxCredential;
use everruns_contracts::typed_id::SessionId;
use everruns_core::McpServerActsAs;
use everruns_core::connection_services::UserConnectionResolver;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::connections;
use crate::providers::OrgMaterial;

/// An org's material whose lookups answer from a script and are recorded.
struct Material {
    reaches: bool,
    answer: Mutex<Option<anyhow::Result<LookupAnswer>>>,
    asked: Mutex<Vec<(String, String, Value)>>,
}

impl Material {
    fn new(reaches: bool, answer: anyhow::Result<LookupAnswer>) -> Arc<Self> {
        Arc::new(Self {
            reaches,
            answer: Mutex::new(Some(answer)),
            asked: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl OrgMaterial for Material {
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
        self.reaches
    }

    async fn lookup(
        &self,
        org: &str,
        command: &str,
        params: Value,
    ) -> anyhow::Result<LookupAnswer> {
        self.asked
            .lock()
            .push((org.to_owned(), command.to_owned(), params));
        self.answer
            .lock()
            .take()
            .unwrap_or_else(|| anyhow::bail!("asked twice"))
    }
}

/// Sandbox keys that answer one fixed key.
struct Keys;

#[async_trait::async_trait]
impl UserConnectionResolver for Keys {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(None)
    }

    async fn get_sandbox_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
        _credential: &SessionSandboxCredential,
    ) -> everruns_contracts::error::Result<Option<String>> {
        Ok(Some("sandbox-key".into()))
    }
}

fn resolver(material: &Arc<Material>, session: SessionId) -> Arc<dyn UserConnectionResolver> {
    let material: Arc<dyn OrgMaterial> = material.clone();
    connections(Arc::new(Keys), Some(&Arc::downgrade(&material)), session)
}

#[tokio::test]
async fn a_connection_token_is_looked_up_for_the_session_as_its_org() {
    let material = Material::new(true, Ok(LookupAnswer::Value(json!("gh-token"))));
    let session = SessionId::new();
    let token = resolver(&material, session)
        .get_connection_token(session, "github")
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("gh-token"));
    let asked = material.asked.lock();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, "org-a");
    assert_eq!(asked[0].1, "worker_get_connection_token");
    assert_eq!(asked[0].2["session_id"], json!(session.to_string()));
    assert_eq!(asked[0].2["provider"], json!("github"));
}

#[tokio::test]
async fn sandbox_keys_and_mcp_grants_stay_on_the_data_plane() {
    let material = Material::new(true, Ok(LookupAnswer::Value(json!("never"))));
    let session = SessionId::new();
    let resolver = resolver(&material, session);
    let credential = SessionSandboxCredential::default();
    assert_eq!(
        resolver
            .get_sandbox_connection_token(session, "daytona", &credential)
            .await
            .unwrap()
            .as_deref(),
        Some("sandbox-key")
    );
    assert_eq!(
        resolver
            .get_mcp_connection_token(session, "mcp_oauth_docs", McpServerActsAs::Service)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        resolver
            .get_connection_token(session, "mcp_oauth_docs")
            .await
            .unwrap(),
        None
    );
    assert!(material.asked.lock().is_empty());
}

#[tokio::test]
async fn a_refusal_and_an_unreachable_control_plane_fail_the_lookup() {
    let refused = Material::new(
        true,
        Ok(LookupAnswer::Error {
            kind: 3,
            message: "Session not found".into(),
        }),
    );
    let session = SessionId::new();
    let error = resolver(&refused, session)
        .get_connection_token(session, "github")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Session not found"), "{error}");

    let down = Material::new(true, Err(anyhow::anyhow!("connection refused")));
    let error = resolver(&down, session)
        .get_tool_approval_policy(session)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("could not be reached"),
        "{error}"
    );
}

#[tokio::test]
async fn a_cell_without_a_control_plane_answers_as_a_host_without_org_settings() {
    let material = Material::new(false, Ok(LookupAnswer::Value(json!("never"))));
    let session = SessionId::new();
    let resolver = resolver(&material, session);
    assert_eq!(
        resolver
            .get_connection_token(session, "github")
            .await
            .unwrap(),
        None
    );
    assert!(
        resolver
            .get_web_search_settings(session)
            .await
            .unwrap()
            .is_none()
    );
    assert!(material.asked.lock().is_empty());
}
