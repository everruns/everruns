use super::*;
use everruns_contracts::runtime::egress::{
    EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use everruns_contracts::runtime::{ServiceApiKeyConnection, UserConnectionResolver};
use everruns_contracts::typed_id::SessionId;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const INBOX: &str = "research@acme.agentmail.to";

/// Hands out a service connection when configured, and records every call
/// so tests can prove the end user's connections are never read.
#[derive(Default)]
struct Resolver {
    service: Option<ServiceApiKeyConnection>,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl UserConnectionResolver for Resolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        provider: &str,
    ) -> everruns_contracts::error::Result<Option<String>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("user_token:{provider}"));
        Ok(Some("end-user-key".into()))
    }

    async fn get_connection_metadata(
        &self,
        _session_id: SessionId,
        provider: &str,
    ) -> everruns_contracts::error::Result<Option<Value>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("user_metadata:{provider}"));
        Ok(Some(json!({ "inbox_id": "end-user@agentmail.to" })))
    }

    async fn get_service_api_key_connection(
        &self,
        _session_id: SessionId,
        provider: &str,
    ) -> everruns_contracts::error::Result<Option<ServiceApiKeyConnection>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("service:{provider}"));
        Ok(self.service.clone())
    }
}

struct Egress {
    status: u16,
    body: Value,
    seen: Mutex<Vec<EgressRequest>>,
}

#[async_trait]
impl EgressService for Egress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        self.seen.lock().unwrap().push(request);
        Ok(EgressResponse {
            status: self.status,
            headers: BTreeMap::new(),
            body: self.body.to_string().into_bytes(),
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unreachable!("the authorize call is buffered")
    }
}

fn service_connection() -> ServiceApiKeyConnection {
    ServiceApiKeyConnection {
        api_key: "am_service_key".into(),
        metadata: Some(json!({ "inbox_id": INBOX })),
    }
}

fn egress(status: u16, body: Value) -> Arc<Egress> {
    Arc::new(Egress {
        status,
        body,
        seen: Mutex::new(Vec::new()),
    })
}

async fn run(
    resolver: Arc<Resolver>,
    egress: Arc<Egress>,
    arguments: Value,
) -> ToolExecutionResult {
    let context = ToolContext::new(SessionId::new())
        .with_connection_resolver(resolver)
        .with_egress_service(egress);
    AgentIdAuthorizeTool
        .execute_with_context(arguments, &context)
        .await
}

fn accepted() -> Arc<Egress> {
    egress(
        202,
        json!({ "api_key_id": "key_123", "instructions": "Continue in the app." }),
    )
}

#[tokio::test]
async fn refuses_without_a_service_connection() {
    let resolver = Arc::new(Resolver::default());
    let egress = accepted();
    let result = run(
        resolver.clone(),
        egress.clone(),
        json!({ "auth_token": "tok" }),
    )
    .await;
    assert!(result.is_error(), "{result:?}");
    assert!(egress.seen.lock().unwrap().is_empty());
    // It asked only for the service connection, and did not fall back.
    assert_eq!(*resolver.calls.lock().unwrap(), vec!["service:agentmail"]);
}

#[tokio::test]
async fn never_reads_the_invoking_users_connections() {
    let resolver = Arc::new(Resolver {
        service: Some(service_connection()),
        ..Resolver::default()
    });
    let egress = accepted();
    let result = run(
        resolver.clone(),
        egress.clone(),
        json!({ "auth_token": "tok" }),
    )
    .await;
    assert!(!result.is_error(), "{result:?}");
    assert_eq!(*resolver.calls.lock().unwrap(), vec!["service:agentmail"]);
    let seen = egress.seen.lock().unwrap();
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer am_service_key")
    );
}

#[tokio::test]
async fn sends_only_the_auth_token_to_the_inbox_authorize_endpoint() {
    let resolver = Arc::new(Resolver {
        service: Some(service_connection()),
        ..Resolver::default()
    });
    let egress = accepted();
    let result = run(
        resolver,
        egress.clone(),
        json!({ "auth_token": " tok-from-waiting-page " }),
    )
    .await;
    assert!(!result.is_error(), "{result:?}");

    let seen = egress.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let request = &seen[0];
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.url,
        "https://api.agentmail.to/v0/inboxes/research%40acme.agentmail.to/authorize"
    );
    assert!(request.dns_pinning_required);
    assert_eq!(request.kind, EgressRequestKind::Integration);
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        body,
        json!({ "auth_token": "tok-from-waiting-page", "accept_disclosure": true })
    );
    let output = format!("{result:?}");
    assert!(output.contains("key_123"));
    assert!(!output.contains("am_service_key"));
}

#[tokio::test]
async fn agentmail_refusals_become_tool_errors_with_its_reason() {
    let resolver = Arc::new(Resolver {
        service: Some(service_connection()),
        ..Resolver::default()
    });
    let egress = egress(
        403,
        json!({ "code": "limit_exceeded", "message": "Too many apps", "fix": "Revoke one" }),
    );
    let result = run(resolver, egress, json!({ "auth_token": "tok" })).await;
    assert!(result.is_error());
    let output = format!("{result:?}");
    assert!(output.contains("limit_exceeded"), "{output}");
    assert!(!output.contains("am_service_key"));
}

#[tokio::test]
async fn a_connection_without_an_inbox_or_an_empty_token_is_refused() {
    let resolver = Arc::new(Resolver {
        service: Some(ServiceApiKeyConnection {
            api_key: "am_service_key".into(),
            metadata: None,
        }),
        ..Resolver::default()
    });
    let egress = accepted();
    assert!(
        run(
            resolver.clone(),
            egress.clone(),
            json!({ "auth_token": "tok" })
        )
        .await
        .is_error()
    );
    assert!(
        run(resolver, egress.clone(), json!({ "auth_token": "  " }))
            .await
            .is_error()
    );
    assert!(egress.seen.lock().unwrap().is_empty());
}
