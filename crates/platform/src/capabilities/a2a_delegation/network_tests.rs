//! SSRF / runtime network-policy proofs for outbound A2A (EVE-1173).

use super::*;
use a2a::{AgentCapabilities, AgentInterface};
use everruns_contracts::typed_id::SessionId;
use everruns_core::deployment::DeploymentGrade;
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::network::controlled_dns_resolver;

/// Local mock agents need `allow_local_urls`; that hatch is gated to
/// `DEPLOYMENT_GRADE=dev`. Set once for the test process — production
/// refusal is covered via `validate_for_grade` without mutating env.
fn ensure_dev_deployment_grade() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        unsafe { std::env::set_var("DEPLOYMENT_GRADE", "dev") };
    });
}
#[test]
fn allow_local_urls_rejected_outside_dev_grade() {
    let agent = ExternalA2aAgentConfig {
        id: "echo".to_string(),
        name: "Echo".to_string(),
        description: None,
        base_url: Some("http://127.0.0.1:1".to_string()),
        agent_card: None,
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: true,
    };
    assert!(!agent.local_urls_permitted_for_grade(DeploymentGrade::Prod));
    assert!(!agent.local_urls_permitted_for_grade(DeploymentGrade::Preview));
    assert!(!agent.local_urls_permitted_for_grade(DeploymentGrade::Poc));
    assert!(agent.local_urls_permitted_for_grade(DeploymentGrade::Dev));
    let err = agent.validate_for_grade(DeploymentGrade::Prod).unwrap_err();
    assert!(
        err.contains("DEPLOYMENT_GRADE=dev"),
        "prod must reject the hatch explicitly: {err}"
    );
}
/// EVE-1173: controlled resolver proves private/loopback/link-local/metadata
/// DNS answers are denied for discovery base URLs and AgentCard interfaces
/// before any TCP connect (destination mock sees zero requests).
#[tokio::test]
async fn controlled_resolver_denies_private_answers_for_base_and_interface_urls() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/agent-card.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string("leaked"))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("leaked"))
        .expect(0)
        .mount(&server)
        .await;

    let listen = reqwest::Url::parse(&server.uri()).unwrap();
    let port = listen.port().unwrap();
    let blocked_answers = ["10.0.0.1", "127.0.0.1", "169.254.169.254", "fe80::1"];

    for blocked in blocked_answers {
        let blocked_ip: IpAddr = blocked.parse().unwrap();
        let resolver = controlled_dns_resolver(move |_host, resolved_port| {
            let addr = SocketAddr::new(blocked_ip, resolved_port);
            async move { Ok(vec![addr]) }
        });

        // Base URL / discovery path.
        let discovery_agent = ExternalA2aAgentConfig {
            id: "probe".to_string(),
            name: "Probe".to_string(),
            description: None,
            base_url: Some(format!("http://rebind.example:{port}")),
            agent_card: None,
            headers: BTreeMap::new(),
            preferred_binding: None,
            poll_interval_ms: None,
            allow_local_urls: false,
        };
        let err = discovery_agent
            .resolve_card_with_resolver(Some(&resolver))
            .await
            .unwrap_err();
        assert!(
            err.contains("unsafe") || err.contains("blocked") || err.contains(blocked),
            "base URL must deny blocked answer {blocked}: {err}"
        );

        // AgentCard interface path (inline card; no discovery fetch).
        let interface_agent = ExternalA2aAgentConfig {
            id: "probe".to_string(),
            name: "Probe".to_string(),
            description: None,
            base_url: None,
            agent_card: Some(AgentCard {
                name: "probe".to_string(),
                description: "probe".to_string(),
                version: "1".to_string(),
                supported_interfaces: vec![AgentInterface::new(
                    format!("http://rebind.example:{port}/jsonrpc"),
                    "JSONRPC",
                )],
                capabilities: AgentCapabilities {
                    streaming: None,
                    push_notifications: None,
                    extensions: None,
                    extended_agent_card: None,
                },
                default_input_modes: vec![],
                default_output_modes: vec![],
                skills: vec![],
                provider: None,
                documentation_url: None,
                icon_url: None,
                security_schemes: None,
                security_requirements: None,
                signatures: None,
            }),
            headers: BTreeMap::new(),
            preferred_binding: Some("JSONRPC".to_string()),
            poll_interval_ms: None,
            allow_local_urls: false,
        };
        let ctx = ToolContext::new(SessionId::new());
        let err = match build_client_with_resolver(&interface_agent, &ctx, Some(&resolver)).await {
            Ok(_) => panic!("interface URL must deny blocked answer {blocked}"),
            Err(err) => err,
        };
        assert!(
            err.contains("unsafe") || err.contains("blocked") || err.contains(blocked),
            "interface URL must deny blocked answer {blocked}: {err}"
        );
    }
}
/// EVE-1173: discovery and interface clients never follow redirects, so a
/// 302 to loopback/metadata cannot move the request off the checked URL.
#[tokio::test]
async fn a2a_clients_do_not_follow_redirects_to_private_destinations() {
    ensure_dev_deployment_grade();

    let private_dest = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("secret"))
        .expect(0)
        .mount(&private_dest)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("secret"))
        .expect(0)
        .mount(&private_dest)
        .await;

    // Discovery: base URL returns 302 to a private/loopback Location.
    let discovery = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/agent-card.json"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/secret", private_dest.uri())),
        )
        .expect(1)
        .mount(&discovery)
        .await;

    let discovery_agent = ExternalA2aAgentConfig {
        id: "redir".to_string(),
        name: "Redir".to_string(),
        description: None,
        base_url: Some(discovery.uri()),
        agent_card: None,
        headers: BTreeMap::new(),
        preferred_binding: None,
        poll_interval_ms: None,
        allow_local_urls: true,
    };
    let err = discovery_agent
        .resolve_card_with_resolver(None)
        .await
        .unwrap_err();
    assert!(
        err.contains("Failed to resolve") || err.contains("agent-card") || err.contains("302"),
        "discovery must fail closed on redirect, not follow it: {err}"
    );

    // Interface: transport POST gets 302 to private; destination untouched.
    let interface = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/secret", private_dest.uri())),
        )
        .expect(1)
        .mount(&interface)
        .await;

    let interface_agent = ExternalA2aAgentConfig {
        id: "redir".to_string(),
        name: "Redir".to_string(),
        description: None,
        base_url: None,
        agent_card: Some(AgentCard {
            name: "redir".to_string(),
            description: "redir".to_string(),
            version: "1".to_string(),
            supported_interfaces: vec![AgentInterface::new(
                format!("{}/jsonrpc", interface.uri()),
                "JSONRPC",
            )],
            capabilities: AgentCapabilities {
                streaming: None,
                push_notifications: None,
                extensions: None,
                extended_agent_card: None,
            },
            default_input_modes: vec![],
            default_output_modes: vec![],
            skills: vec![],
            provider: None,
            documentation_url: None,
            icon_url: None,
            security_schemes: None,
            security_requirements: None,
            signatures: None,
        }),
        headers: BTreeMap::new(),
        preferred_binding: Some("JSONRPC".to_string()),
        poll_interval_ms: None,
        allow_local_urls: true,
    };
    let ctx = ToolContext::new(SessionId::new());
    let client = build_client(&interface_agent, &ctx).await.unwrap();
    let send_err = client
        .send_message(&send_request("hi", None, None, true))
        .await
        .unwrap_err();
    assert!(
        !send_err.to_string().is_empty(),
        "transport must surface the unfollowed redirect as an error"
    );
}
