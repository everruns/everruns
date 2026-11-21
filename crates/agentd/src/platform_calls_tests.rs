//! Platform commands through the control plane's commands server: one MCP
//! tool call with a session token naming the turn's person, served by the
//! core handler a control plane runs.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, Weak};

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use everruns_contracts::dataplane::Connection;
use everruns_contracts::dataplane_seal::Key;
use everruns_contracts::runtime::capabilities::injected::{
    InjectedCapabilityDefinition, PLATFORM_COMMAND_TOOL, PLATFORM_COMMANDS, injected_capability_id,
};
use everruns_contracts::tools::ToolExecutionResult;
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::mcp::injected_server::{
    InjectedCapabilityService, VerifiedSession, handle_injected_request,
};
use everruns_core::mcp::session_token::{SessionTokenSigner, SessionTokenVerifier};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::Tool;
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::mcp::{ControllerServer, OrgMcp};
use crate::providers::OrgMaterial;

// A public address literal: checked without DNS; the egress below answers.
const URL: &str = "https://93.184.215.14/api/v1/injected/platform_commands/mcp";
const KEY: Key = [5u8; 32];
const ORG: &str = "org-a";

/// The control plane's command: says who ran what.
struct Command(Option<String>);

#[async_trait]
impl Tool for Command {
    fn name(&self) -> &str {
        PLATFORM_COMMAND_TOOL
    }
    fn description(&self) -> &str {
        "run"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object" })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        match &self.0 {
            Some(person) => ToolExecutionResult::success(Value::String(format!(
                "{person} ran {} {}",
                arguments["operation"].as_str().unwrap_or_default(),
                arguments["arguments"]
            ))),
            None => ToolExecutionResult::tool_error("no person to act as"),
        }
    }
}

struct Commands(InjectedCapabilityDefinition);

#[async_trait]
impl InjectedCapabilityService for Commands {
    fn definition(&self) -> &InjectedCapabilityDefinition {
        &self.0
    }
    async fn tools(&self, session: &VerifiedSession) -> AnyResult<Vec<Box<dyn Tool>>> {
        Ok(vec![Box::new(Command(
            session.claims.act.as_ref().map(|act| act.sub.clone()),
        ))])
    }
    async fn tool_context(&self, session: &VerifiedSession) -> AnyResult<ToolContext> {
        Ok(ToolContext::new(SessionId::from_uuid(session.session_id)))
    }
}

struct ControlPlane {
    service: Commands,
    verifier: SessionTokenVerifier,
}

#[async_trait]
impl EgressService for ControlPlane {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        assert_eq!(request.url, URL);
        let bearer = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .and_then(|(_, value)| value.strip_prefix("Bearer "))
            .unwrap_or_default()
            .to_string();
        let Ok(claims) =
            self.verifier
                .verify(&bearer, URL, &injected_capability_id(PLATFORM_COMMANDS))
        else {
            return Ok(EgressResponse {
                status: 401,
                headers: BTreeMap::new(),
                body: Vec::new(),
            });
        };
        let session = VerifiedSession::from_claims(claims).unwrap();
        let message: Value = serde_json::from_slice(&request.body).unwrap();
        Ok(
            match handle_injected_request(&self.service, &session, message).await {
                Some(reply) => EgressResponse {
                    status: 200,
                    headers: [("content-type".to_string(), "application/json".to_string())].into(),
                    body: serde_json::to_vec(&reply).unwrap(),
                },
                None => EgressResponse {
                    status: 202,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                },
            },
        )
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter([Ok(response.body)])),
        })
    }
}

/// An executor running one session, whose open turn a person may have sent.
struct Executor {
    session: SessionId,
    sender: Mutex<Option<String>>,
    mcp: OnceLock<Arc<OrgMcp>>,
}

#[async_trait]
impl OrgMaterial for Executor {
    fn tenant(&self, session: SessionId) -> Option<String> {
        (session == self.session).then(|| ORG.to_string())
    }
    async fn connection(&self, _: &str, _: &str) -> anyhow::Result<Option<Connection>> {
        Ok(None)
    }
    async fn secret(&self, _: &str, _: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("no secrets")
    }
    fn data_plane_key(&self) -> Option<Key> {
        Some(KEY)
    }
    fn turn_sender(&self, _: SessionId) -> Option<String> {
        self.sender.lock().clone()
    }
    fn mcp(&self) -> Option<Arc<OrgMcp>> {
        self.mcp.get().cloned()
    }
    fn looks_up(&self) -> bool {
        true
    }
    async fn lookup(
        &self,
        _: &str,
        command: &str,
        _: Value,
    ) -> anyhow::Result<everruns_contracts::host_calls::HostCallAnswer> {
        panic!("platform command made host call {command}")
    }
}

#[tokio::test]
async fn platform_commands_run_on_the_control_plane_as_the_turns_person() {
    let signer = SessionTokenSigner::from_pkcs8(
        everruns_contracts::dataplane_seal::SESSION_TOKEN_ISSUER,
        &everruns_contracts::dataplane_seal::session_token_pkcs8(&KEY),
    )
    .unwrap();
    let definition = serde_json::from_value(json!({
        "name": PLATFORM_COMMANDS,
        "description": "Platform commands",
        "server": { "url": URL },
    }))
    .unwrap();
    let cp = Arc::new(ControlPlane {
        service: Commands(definition),
        verifier: SessionTokenVerifier::from_jwks(&signer.jwks(), vec![signer.issuer().into()])
            .unwrap(),
    });
    let session = SessionId::new();
    let executor = Arc::new(Executor {
        session,
        sender: Mutex::new(Some("usr_1".into())),
        mcp: OnceLock::new(),
    });
    let material: Arc<dyn OrgMaterial> = executor.clone();
    let weak: Weak<dyn OrgMaterial> = Arc::downgrade(&material);
    let mcp = Arc::new(OrgMcp::new(cp.clone()));
    mcp.bind(weak.clone());
    let _ = executor.mcp.set(mcp);

    let lookups = crate::lookups::org_lookups(Some(&weak), session).unwrap();
    let server = ControllerServer::for_session(Some(&weak), session, PLATFORM_COMMANDS, URL);
    let calls = super::PlatformCalls::new(lookups, session, server);

    // Outside a turn: refused before anything is sent.
    assert!(calls.call("discover", json!({})).await.is_err());

    let turn = calls.bound(MessageId::new());
    let output = turn
        .call("execute", json!({ "command": "agents list" }))
        .await
        .unwrap();
    assert!(output.starts_with("usr_1 ran execute"), "{output}");
    assert!(output.contains("agents list"), "{output}");

    // A turn no person opened: the control plane refuses, as the tool's error.
    *executor.sender.lock() = None;
    let error = turn.call("query", json!({})).await.unwrap_err();
    assert!(error.to_string().contains("no person to act as"), "{error}");
}
