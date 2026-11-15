#![cfg(feature = "github")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use everruns::capability::{self, Context, Definition};
use everruns::github::{GitHub, GitHubTokenProvider, GitHubTokenRequest};
use everruns::{Agent, ContentPart, Engine, IntoCapability, LlmSimConfig, Model};
use everruns_llmsim::{SimToolCall, SimTurn};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

fn definition(github: GitHub) -> Definition {
    github.into_capability().into_parts().definition.unwrap()
}

async fn invoke(
    definition: &Definition,
    name: &str,
    arguments: Value,
) -> Result<Value, capability::Error> {
    definition
        .tools()
        .iter()
        .find(|tool| tool.spec().name() == name)
        .unwrap()
        .invoke(arguments, Context::new(name, "session", "workspace"))
        .await
}

#[test]
fn writes_require_explicit_options_and_secrets_are_not_configuration() {
    let github = GitHub::new("sentinel-github-token").allow_pull_requests(true);
    assert!(!format!("{github:?}").contains("sentinel-github-token"));
    let spec = github.into_capability();
    assert_eq!(spec.capability_ref().id(), "github");
    assert!(!format!("{spec:?}").contains("sentinel-github-token"));
    let readonly = spec.into_parts().definition.unwrap();
    assert_eq!(readonly.tools().len(), 5);
    assert!(
        readonly
            .tools()
            .iter()
            .all(|tool| tool.spec().hints().readonly == Some(true))
    );
    let write = definition(GitHub::new("token").allow_writes(true));
    assert_eq!(write.tools().len(), 8);
    assert!(
        !write
            .tools()
            .iter()
            .any(|tool| tool.spec().name() == "create_github_pull_request")
    );
    let all = definition(
        GitHub::new("token")
            .allow_writes(true)
            .allow_pull_requests(true),
    );
    assert_eq!(all.tools().len(), 9);
}

#[tokio::test]
async fn real_vendor_operation_runs_through_engine_without_server_or_credentials_in_events() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/app/pulls/7"))
        .and(header("Authorization", "Bearer sentinel-github-token"))
        .and(header("Accept", "application/vnd.github.diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string("+ agent fix\n"))
        .expect(1)
        .mount(&server)
        .await;
    let agent = Agent::builder()
        .instructions("Read the GitHub diff before reviewing.")
        .model(Model::simulated_with_config(LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "get_github_pull_request_diff".into(),
                arguments: json!({"repo": "acme/app", "number": 7}),
                id: Some("diff_1".into()),
            }]),
            SimTurn::Assistant("Reviewed the fix.".into()),
        ])))
        .capability(GitHub::new("sentinel-github-token").api_base(server.uri()))
        .build()
        .unwrap();
    assert!(!format!("{agent:?}").contains("sentinel-github-token"));
    let session = Engine::new().create(agent);
    let turn = session.send_and_wait("Review PR 7").await.unwrap();
    assert!(turn.success);
    assert_eq!(turn.tool_calls, 1);
    let history = session.history().page().await.unwrap();
    let result = history
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| match part {
            ContentPart::ToolResult(result) if result.tool_call_id == "diff_1" => Some(result),
            _ => None,
        })
        .unwrap();
    assert!(result.error.is_none());
    assert_eq!(result.result.as_ref().unwrap()["diff"], "+ agent fix\n");
    assert!(!format!("{:?}", history.messages).contains("sentinel-github-token"));
    let context = session.inspect().await.unwrap();
    assert!(!format!("{context:?}").contains("sentinel-github-token"));
}

struct RotatingTokens(Arc<Mutex<Vec<GitHubTokenRequest>>>);
#[capability::async_trait]
impl GitHubTokenProvider for RotatingTokens {
    async fn token(&self, request: &GitHubTokenRequest) -> Result<String, String> {
        let mut calls = self.0.lock().unwrap();
        calls.push(request.clone());
        Ok(format!("rotating-token-{}", calls.len()))
    }
}

#[tokio::test]
async fn credentials_are_resolved_per_call_with_repository_and_permissions() {
    let server = MockServer::start().await;
    for n in 1..=2 {
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7"))
            .and(header(
                "Authorization",
                format!("Bearer rotating-token-{n}"),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string("diff"))
            .expect(1)
            .mount(&server)
            .await;
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let definition = definition(
        GitHub::with_token_provider(RotatingTokens(requests.clone())).api_base(server.uri()),
    );
    for _ in 0..2 {
        invoke(
            &definition,
            "get_github_pull_request_diff",
            json!({"repo": "acme/app", "number": 7}),
        )
        .await
        .unwrap();
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.repository.as_deref() == Some("acme/app")
                && request.permissions == [("pull_requests".into(), "read".into())].into())
    );
}

#[tokio::test]
async fn invalid_targets_and_forbidden_approval_do_not_resolve_credentials() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let definition = definition(
        GitHub::with_token_provider(RotatingTokens(requests.clone())).allow_writes(true),
    );
    for (tool, arguments) in [
        (
            "get_github_pull_request",
            json!({"repo": "../bad", "number": 7}),
        ),
        (
            "get_github_pull_request_diff",
            json!({"repo": "acme/app", "number": 0}),
        ),
        (
            "submit_github_pull_request_review",
            json!({"repo": "acme/app", "number": 7, "event": "APPROVE", "body": "yes"}),
        ),
        (
            "upsert_github_comment",
            json!({"repo": "acme/app", "number": 7, "body": "ok", "marker": "../bad"}),
        ),
    ] {
        assert!(invoke(&definition, tool, arguments).await.is_err());
    }
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upstream_errors_cannot_echo_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad sentinel-github-token"))
        .expect(1)
        .mount(&server)
        .await;
    let definition = definition(GitHub::new("sentinel-github-token").api_base(server.uri()));
    let error = invoke(
        &definition,
        "get_github_pull_request_diff",
        json!({"repo":"acme/app", "number":7}),
    )
    .await
    .unwrap_err();
    assert!(error.message().contains("401"));
    assert!(!format!("{error:?}").contains("sentinel-github-token"));
}

#[tokio::test]
async fn private_issue_policy_is_enforced_before_any_write() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/app"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"full_name":"acme/app", "private":false})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let definition = definition(
        GitHub::new("token")
            .api_base(server.uri())
            .allow_writes(true)
            .private_issues_only(true),
    );
    let error = invoke(
        &definition,
        "upsert_github_issue",
        json!({"repo":"acme/app", "fingerprint":"finding", "title":"Issue", "body":"Evidence"}),
    )
    .await
    .unwrap_err();
    assert!(error.message().contains("public"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

struct FailingTokens;
#[capability::async_trait]
impl GitHubTokenProvider for FailingTokens {
    async fn token(&self, _: &GitHubTokenRequest) -> Result<String, String> {
        Err("sentinel-private-refresh-token".into())
    }
}

#[tokio::test]
async fn provider_errors_and_empty_tokens_fail_without_network_or_environment_fallback() {
    let server = MockServer::start().await;
    for github in [GitHub::with_token_provider(FailingTokens), GitHub::new("")] {
        let definition = definition(github.api_base(server.uri()));
        let error = invoke(
            &definition,
            "get_github_pull_request_diff",
            json!({"repo":"acme/app", "number":7}),
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:?}").contains("sentinel-private-refresh-token"));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
