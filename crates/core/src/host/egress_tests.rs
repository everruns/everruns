use super::*;
use crate::EgressRequestKind;
use crate::network_access::NetworkAccessList;
use futures::StreamExt;
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn direct_service_sends_json_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/test"))
        .and(header("Authorization", "Bearer test"))
        .and(body_json(json!({"ok": true})))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({
                    "id": "response_123"
                }))
                .insert_header("X-Request-Id", "request-42"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let response = DirectEgressService::new()
        .send(
            EgressRequest::new(
                "POST",
                format!("{}/v1/test", server.uri()),
                EgressRequestKind::Capability,
            )
            .header("Authorization", "Bearer test")
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&json!({"ok": true})).unwrap()),
        )
        .await
        .unwrap();

    assert_eq!(response.status, 201);
    assert_eq!(
        response.headers.get("x-request-id").map(String::as_str),
        Some("request-42")
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["id"],
        "response_123"
    );
}

#[tokio::test]
async fn direct_service_enforces_network_access() {
    let error = DirectEgressService::new()
        .send(
            EgressRequest::new(
                "GET",
                "https://blocked.example.com/path",
                EgressRequestKind::Capability,
            )
            .network_access(Some(NetworkAccessList::allow_only(["allowed.example.com"]))),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
}

#[tokio::test]
async fn system_allowlist_blocks_unlisted_hosts() {
    use crate::SystemAllowlist;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/blocked"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let allowlist = SystemAllowlist::from_toml(
        r#"
        [groups.test]
        allowed = ["allowed.example.com"]
        "#,
    )
    .unwrap();

    let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
    let error = service
        .send(EgressRequest::new(
            "GET",
            format!("{}/blocked", server.uri()),
            EgressRequestKind::Capability,
        ))
        .await
        .unwrap_err();

    assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
}

#[tokio::test]
async fn system_allowlist_cannot_be_overridden_by_request_acl() {
    use crate::SystemAllowlist;
    // The system allowlist is a hard ceiling: even a request whose own
    // network_access explicitly permits the host must still be denied when
    // the system allowlist does not list it. Sessions can narrow, never widen.
    let allowlist = SystemAllowlist::from_toml(
        r#"
        [groups.test]
        allowed = ["allowed.example.com"]
        "#,
    )
    .unwrap();

    let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
    let error = service
        .send(
            EgressRequest::new(
                "GET",
                "https://blocked.example.com/path",
                EgressRequestKind::Capability,
            )
            // A maximally-permissive per-request ACL that allows the host.
            .network_access(Some(NetworkAccessList::allow_only(["blocked.example.com"]))),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
}

#[tokio::test]
async fn system_allowlist_permits_listed_hosts() {
    use crate::SystemAllowlist;
    let server = MockServer::start().await;
    let host = reqwest::Url::parse(&server.uri())
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();
    Mock::given(method("GET"))
        .and(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let allowlist =
        SystemAllowlist::from_toml(&format!("[groups.test]\nallowed = [\"{host}\"]\n")).unwrap();
    let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
    let response = service
        .send(EgressRequest::new(
            "GET",
            format!("{}/ok", server.uri()),
            EgressRequestKind::Capability,
        ))
        .await
        .unwrap();

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn dns_pinning_blocks_loopback_after_request_policy_passes() {
    let error = DirectEgressService::new()
        .send(
            EgressRequest::new(
                "GET",
                "http://127.0.0.1/latest/meta-data",
                EgressRequestKind::Capability,
            )
            .network_access(Some(NetworkAccessList::allow_only(["127.0.0.1"])))
            .require_dns_pinning(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    assert!(error.to_string().contains("private/internal address"));
}

/// EVE-1154: bashkit can hand the egress boundary an unpinned hostname
/// after its DNS precheck fails open. With `require_dns_pinning`, a
/// subsequent private/link-local/loopback answer must be refused before
/// any TCP connect — proved here with a controlled resolver aimed at a
/// live mock server that must see zero requests.
#[tokio::test]
async fn require_dns_pinning_denies_private_answers_before_connect() {
    use std::net::{IpAddr, SocketAddr};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/secret"))
        .respond_with(ResponseTemplate::new(200).set_body_string("leaked"))
        .expect(0)
        .mount(&server)
        .await;

    let listen = reqwest::Url::parse(&server.uri()).unwrap();
    let port = listen.port().unwrap();
    for blocked in ["127.0.0.1", "169.254.169.254", "10.0.0.1"] {
        let blocked_ip: IpAddr = blocked.parse().unwrap();
        let service = DirectEgressService::new().with_dns_resolver(move |_host, resolved_port| {
            let addr = SocketAddr::new(blocked_ip, resolved_port);
            async move { Ok(vec![addr]) }
        });
        let error = service
            .send(
                EgressRequest::new(
                    "GET",
                    format!("http://rebind.example:{port}/secret"),
                    EgressRequestKind::Capability,
                )
                .network_access(Some(NetworkAccessList::allow_only(["rebind.example"])))
                // Same flag BashkitEgressTransport sets when pins are empty.
                .require_dns_pinning(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, EgressError::NetworkAccessDenied { .. }),
            "blocked answer {blocked} must deny: {error}"
        );
        assert!(
            error.to_string().contains(blocked)
                || error.to_string().contains("blocked address")
                || error.to_string().contains("private"),
            "denial must mention the blocked answer {blocked}: {error}"
        );
    }
}

#[tokio::test]
async fn dns_pinning_does_not_run_before_system_allowlist_denial() {
    use crate::SystemAllowlist;
    let allowlist = SystemAllowlist::from_toml(
        r#"
        [groups.test]
        allowed = ["allowed.example.com"]
        "#,
    )
    .unwrap();

    let blocked_url = "https://blocked.invalid/path";
    let error = DirectEgressService::new()
        .with_system_allowlist(Some(Arc::new(allowlist)))
        .send(
            EgressRequest::new("GET", blocked_url, EgressRequestKind::Capability)
                .network_access(Some(NetworkAccessList::allow_only(["blocked.invalid"])))
                .require_dns_pinning(),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, EgressError::NetworkAccessDenied { ref url } if url == blocked_url));
}

#[tokio::test]
async fn required_signing_fails_when_no_signer_is_configured() {
    let error = DirectEgressService::new()
        .send(
            EgressRequest::new("GET", "https://example.com", EgressRequestKind::Capability)
                .signing(EgressSigning::Required),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, EgressError::SigningUnavailable));
}

#[tokio::test]
async fn direct_service_does_not_follow_redirects() {
    let redirect_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/secret"))
        .respond_with(ResponseTemplate::new(200).set_body_string("secret"))
        .expect(0)
        .mount(&redirect_server)
        .await;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/secret", redirect_server.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;

    let response = DirectEgressService::new()
        .send(EgressRequest::new(
            "GET",
            format!("{}/start", server.uri()),
            EgressRequestKind::Capability,
        ))
        .await
        .unwrap();

    assert_eq!(response.status, 302);
}

#[tokio::test]
async fn direct_service_streams_response_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("data: one\n\ndata: two\n\n", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut response = DirectEgressService::new()
        .send_stream(EgressRequest::new(
            "GET",
            format!("{}/stream", server.uri()),
            EgressRequestKind::Capability,
        ))
        .await
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.body.next().await {
        body.extend(chunk.unwrap());
    }
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "data: one\n\ndata: two\n\n"
    );
}
#[tokio::test]
async fn merged_network_policy_denies_escaped_urls_before_transport() {
    use crate::network_access::merge_network_access;
    let service = DirectEgressService::new();
    for (parent, child, target) in [
        (
            "*.example.com",
            "https://outside.invalid/path.example.com",
            "https://outside.invalid/path.example.com",
        ),
        (
            "https://api.example.com/v1/",
            "https://api.example.com/v1/../admin",
            "https://api.example.com/admin",
        ),
        (
            "https://",
            "https://outside.invalid/data",
            "https://outside.invalid/data",
        ),
    ] {
        let parent = NetworkAccessList::allow_only([parent]);
        let child = NetworkAccessList::allow_only([child]);
        let policy = merge_network_access(Some(&parent), Some(&child));
        for streaming in [false, true] {
            let request = EgressRequest::new("GET", target, EgressRequestKind::Capability)
                .network_access(policy.clone());
            let error = if streaming {
                match service.send_stream(request).await {
                    Err(error) => error,
                    Ok(_) => panic!("streaming request escaped policy: {target}"),
                }
            } else {
                service.send(request).await.unwrap_err()
            };
            assert!(matches!(error, EgressError::NetworkAccessDenied { url } if url == target));
        }
    }
}

#[test]
fn audit_record_carries_scope_and_hides_query() {
    let session_id = everruns_contracts::typed_id::SessionId::new();
    let request = EgressRequest::new(
        "post",
        "https://hooks.example.com/hook?token=sekret",
        EgressRequestKind::Other("plugin".into()),
    )
    .scope(crate::EgressScope {
        org_id: Some(everruns_contracts::typed_id::DEFAULT_ORG_ID),
        session_id: Some(session_id),
    })
    .body(b"12345".to_vec());
    let audit = EgressAudit::start(&request, PolicyDecision::NoPolicy);
    assert_eq!(audit.kind, "other:plugin");
    assert_eq!(audit.method, "POST");
    assert_eq!(audit.host, "hooks.example.com");
    assert_eq!(audit.path, "/hook");
    assert_eq!(audit.query_len, "token=sekret".len());
    assert_eq!(audit.request_bytes, 5);
    assert_eq!(
        audit.org_id,
        Some(everruns_contracts::typed_id::DEFAULT_ORG_ID.to_string())
    );
    assert_eq!(audit.session_id, Some(session_id.to_string()));
    let debug = format!(
        "{} {} {} {:?} {:?}",
        audit.kind, audit.host, audit.path, audit.org_id, audit.session_id
    );
    assert!(
        !debug.contains("sekret"),
        "query values must not be recorded"
    );
}

#[tokio::test]
async fn scoped_service_overwrites_caller_scope() {
    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<Option<crate::EgressScope>>>);
    #[async_trait]
    impl EgressService for Recorder {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            self.0.lock().unwrap().push(request.scope);
            Err(EgressError::Transport("recorded".into()))
        }
        async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            self.0.lock().unwrap().push(request.scope);
            Err(EgressError::Transport("recorded".into()))
        }
    }
    let recorder = Arc::new(Recorder::default());
    let host_scope = crate::EgressScope {
        org_id: Some(everruns_contracts::typed_id::DEFAULT_ORG_ID),
        session_id: Some(everruns_contracts::typed_id::SessionId::new()),
    };
    let scoped = crate::ScopedEgressService::new(recorder.clone(), host_scope.clone());
    let forged = EgressRequest::new("GET", "https://a.test/", EgressRequestKind::Capability)
        .scope(crate::EgressScope::default());
    let _ = scoped.send(forged.clone()).await;
    let _ = scoped.send_stream(forged).await;
    assert_eq!(
        *recorder.0.lock().unwrap(),
        vec![Some(host_scope.clone()), Some(host_scope)]
    );
}

#[test]
fn audit_outcome_labels() {
    assert_eq!(
        audit_outcome(Ok((200, Some(3)))),
        ("allowed", Some(200), Some(3), None)
    );
    assert_eq!(
        audit_outcome(Err(&EgressError::NetworkAccessDenied {
            url: "https://x.test/?k=v".into()
        })),
        ("denied", None, None, None)
    );
    assert_eq!(
        audit_outcome(Err(&EgressError::Transport("reset".into()))),
        (
            "failed",
            None,
            None,
            Some("Outbound transport error: reset".into())
        )
    );
}

fn curated_writes_service(open_reads_per_minute: u32) -> DirectEgressService {
    use everruns_contracts::runtime::EgressPolicyMode;
    let allowlist =
        crate::SystemAllowlist::from_toml("[groups.test]\nallowed = [\"allowed.example.com\"]\n")
            .unwrap();
    DirectEgressService::new()
        .with_system_policy(Some(Arc::new(SystemEgressPolicy::new(
            EgressPolicyMode::CuratedWrites,
            Arc::new(allowlist),
            vec!["*.webhook.site".to_string()],
        ))))
        .with_open_read_limit(open_reads_per_minute)
}

fn scoped_request(method: &str, url: &str, org: &str) -> EgressRequest {
    EgressRequest::new(method, url, EgressRequestKind::Capability).scope(crate::EgressScope {
        org_id: Some(org.parse().unwrap()),
        session_id: None,
    })
}

#[tokio::test]
async fn curated_writes_lets_reads_through_and_gates_writes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let service = curated_writes_service(10);
    // Wiremock listens on an IP literal, which open reads refuse; that
    // refusal proves the read path ran (a write would say not allowlisted).
    let read = service
        .send(EgressRequest::new(
            "GET",
            format!("{}/page", server.uri()),
            EgressRequestKind::Capability,
        ))
        .await
        .unwrap_err();
    assert!(matches!(read, EgressError::NetworkAccessDenied { .. }));

    let policy = service.system_policy.clone().unwrap();
    let read = EgressRequest::new(
        "GET",
        "https://blog.example.net/p",
        EgressRequestKind::Capability,
    );
    assert_eq!(
        policy.check(&read.url, request_access(&read), None),
        Ok(EgressPolicyGrant::OpenRead)
    );
    for request in [
        EgressRequest::new(
            "POST",
            "https://blog.example.net/p",
            EgressRequestKind::Capability,
        ),
        EgressRequest::new(
            "GET",
            "https://blog.example.net/p",
            EgressRequestKind::Capability,
        )
        .body(b"x".to_vec()),
        EgressRequest::new("GET", "https://blog.example.net/p", EgressRequestKind::Mcp),
        EgressRequest::new(
            "GET",
            "https://blog.example.net/p",
            EgressRequestKind::Integration,
        ),
    ] {
        let error = service.send(request).await.unwrap_err();
        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }
    let denied = service
        .send(EgressRequest::new(
            "GET",
            "https://webhook.site/abc",
            EgressRequestKind::Capability,
        ))
        .await
        .unwrap_err();
    assert!(matches!(denied, EgressError::NetworkAccessDenied { .. }));
}

impl DirectEgressService {
    /// The policy step of `prepare_request`, without DNS or transport.
    async fn admit(&self, request: &EgressRequest) -> EgressResult<()> {
        let decision = self.decide_policy(request).await;
        self.validate_request(request, decision)
    }
}

#[tokio::test]
async fn open_reads_are_metered_per_org() {
    let service = curated_writes_service(2);
    for org in [
        "org_00000000000000000000000000000001",
        "org_00000000000000000000000000000002",
    ] {
        for _ in 0..2 {
            service
                .admit(&scoped_request("GET", "https://blog.example.net/", org))
                .await
                .unwrap();
        }
        let error = service
            .admit(&scoped_request("GET", "https://blog.example.net/", org))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, EgressError::NetworkAccessDenied { url } if url.contains("rate limit")),
            "{error}"
        );
        // Allowlisted traffic is never metered.
        service
            .admit(&scoped_request("GET", "https://allowed.example.com/", org))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn audit_record_names_the_policy_outcome() {
    let service = curated_writes_service(10);
    for (method, url, expected) in [
        ("GET", "https://blog.example.net/", "open_read"),
        ("POST", "https://allowed.example.com/", "allowlisted"),
        (
            "POST",
            "https://blog.example.net/",
            "denied:not_allowlisted",
        ),
        ("GET", "https://webhook.site/x", "denied:denylisted"),
    ] {
        let request = EgressRequest::new(method, url, EgressRequestKind::Capability);
        let decision = service.decide_policy(&request).await;
        assert_eq!(
            EgressAudit::start(&request, decision).policy,
            expected,
            "{method} {url}"
        );
    }
    let request = EgressRequest::new("GET", "https://x.test/", EgressRequestKind::Capability);
    let decision = DirectEgressService::new().decide_policy(&request).await;
    assert_eq!(EgressAudit::start(&request, decision).policy, "none");
}

const ORG_A: &str = "org_00000000000000000000000000000001";
const ORG_B: &str = "org_00000000000000000000000000000002";

fn org_extension_resolver(
    patterns: &[&str],
) -> Arc<crate::host::org_egress_allowlist::tests::FixedResolver> {
    let mut resolver = crate::host::org_egress_allowlist::tests::FixedResolver::default();
    resolver.answers.insert(
        ORG_A.parse().unwrap(),
        Ok(crate::host::org_egress_allowlist::tests::extension_grant(
            NetworkAccessList::allow_only(patterns.iter().copied()),
        )),
    );
    // ORG_B has no grant: the resolver answers with an empty one.
    resolver
        .answers
        .insert(ORG_B.parse().unwrap(), Ok(Default::default()));
    Arc::new(resolver)
}

fn lookups(resolver: &crate::host::org_egress_allowlist::tests::FixedResolver) -> usize {
    resolver.lookups.load(std::sync::atomic::Ordering::SeqCst)
}

#[tokio::test]
async fn granted_org_extension_admits_writes_for_that_org_only() {
    let resolver = org_extension_resolver(&["api.customer.example"]);
    let service = curated_writes_service(10).with_org_allowlist(resolver.clone());
    let url = "https://api.customer.example/v1/items";

    let granted = scoped_request("POST", url, ORG_A);
    assert_eq!(
        service.decide_policy(&granted).await,
        PolicyDecision::AllowlistedOrg
    );
    service.admit(&granted).await.unwrap();
    assert_eq!(
        EgressAudit::start(&granted, PolicyDecision::AllowlistedOrg).policy,
        "allowlisted_org"
    );

    for request in [
        scoped_request("POST", url, ORG_B),
        EgressRequest::new("POST", url, EgressRequestKind::Capability),
        EgressRequest::new("POST", url, EgressRequestKind::Mcp).scope(crate::EgressScope {
            org_id: Some(ORG_B.parse().unwrap()),
            session_id: None,
        }),
    ] {
        let error = service.admit(&request).await.unwrap_err();
        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }
    // The extension does not leak to other hosts of the same org.
    assert!(
        service
            .admit(&scoped_request(
                "POST",
                "https://other.customer.example/",
                ORG_A
            ))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_deny_list_beats_an_org_extension_without_a_lookup() {
    let resolver = org_extension_resolver(&["*.webhook.site", "api.customer.example"]);
    let service = curated_writes_service(10).with_org_allowlist(resolver.clone());
    let denied = scoped_request("POST", "https://x.webhook.site/collect", ORG_A);
    assert_eq!(
        service.decide_policy(&denied).await,
        PolicyDecision::Denied(EgressPolicyDenial::Denylisted)
    );
    assert!(service.admit(&denied).await.is_err());
    assert_eq!(
        lookups(&resolver),
        0,
        "deny list decides before the resolver"
    );
}

#[tokio::test]
async fn allowlisted_traffic_never_consults_the_resolver() {
    let resolver = org_extension_resolver(&["api.customer.example"]);
    let service = curated_writes_service(10).with_org_allowlist(resolver.clone());
    service
        .admit(&scoped_request(
            "POST",
            "https://allowed.example.com/",
            ORG_A,
        ))
        .await
        .unwrap();
    assert_eq!(lookups(&resolver), 0);
    // An open read asks once whether the org is open (it is not).
    service
        .admit(&scoped_request("GET", "https://blog.example.net/", ORG_A))
        .await
        .unwrap();
    assert_eq!(lookups(&resolver), 1);

    // Repeated denials hit the cache, not the resolver.
    for _ in 0..3 {
        let _ = service
            .admit(&scoped_request("POST", "https://blog.example.net/", ORG_A))
            .await;
    }
    assert_eq!(lookups(&resolver), 1);
}

#[tokio::test]
async fn a_failing_resolver_fails_closed() {
    let mut resolver = crate::host::org_egress_allowlist::tests::FixedResolver::default();
    resolver.answers.insert(
        ORG_A.parse().unwrap(),
        Err("control plane unreachable".into()),
    );
    let service = curated_writes_service(10).with_org_allowlist(Arc::new(resolver));
    let error = service
        .admit(&scoped_request(
            "POST",
            "https://api.customer.example/",
            ORG_A,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
}

#[tokio::test]
async fn tenant_requests_go_through_the_proxy_with_a_signed_token() {
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let proxy_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/through-proxy"))
        .and(header_exists("proxy-authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_string("proxied"))
        .mount(&proxy_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/direct"))
        .respond_with(ResponseTemplate::new(200).set_body_string("direct"))
        .mount(&proxy_server)
        .await;
    let proxy = super::super::TenantEgressProxy::new(
        &proxy_server.uri(),
        b"0123456789abcdef0123456789abcdef".to_vec(),
    )
    .unwrap();
    let service = DirectEgressService::new().with_tenant_proxy(Some(proxy));

    let tenant = scoped_request("GET", "http://target.invalid/through-proxy", ORG_A);
    let response = service.send(tenant).await.unwrap();
    assert_eq!(response.body, b"proxied");
    let received = proxy_server.received_requests().await.unwrap();
    let auth = received[0]
        .headers
        .get("proxy-authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(auth.starts_with(&format!("Bearer v1.{ORG_A}.-.")), "{auth}");

    // Host-owned traffic never uses the proxy.
    let provider = EgressRequest::new(
        "GET",
        format!("{}/direct", proxy_server.uri()),
        EgressRequestKind::Provider,
    );
    assert_eq!(service.send(provider).await.unwrap().body, b"direct");
    let received = proxy_server.received_requests().await.unwrap();
    assert!(received[1].headers.get("proxy-authorization").is_none());
}

fn open_org_resolver() -> Arc<crate::host::org_egress_allowlist::tests::FixedResolver> {
    let mut resolver = crate::host::org_egress_allowlist::tests::FixedResolver::default();
    resolver.answers.insert(
        ORG_A.parse().unwrap(),
        Ok(everruns_contracts::runtime::OrgEgressGrant {
            open: true,
            extension: None,
        }),
    );
    Arc::new(resolver)
}

#[tokio::test]
async fn an_open_org_reaches_any_host_the_deny_list_allows() {
    let service = curated_writes_service(1).with_org_allowlist(open_org_resolver());
    let long_url = format!("https://blog.example.net/?q={}", "a".repeat(4096));
    for (method, url) in [
        ("POST", "https://hooks.customer.example/in"),
        ("PUT", "https://blog.example.net/"),
        ("GET", long_url.as_str()),
        ("POST", "https://203.0.113.7/api"),
    ] {
        let request = scoped_request(method, url, ORG_A);
        assert_eq!(
            service.decide_policy(&request).await,
            PolicyDecision::OpenOrg,
            "{method} {url}"
        );
        service.admit(&request).await.unwrap();
    }
    // Reads are not held to the open-read meter (limit 1 above).
    for _ in 0..3 {
        service
            .admit(&scoped_request("GET", "https://blog.example.net/", ORG_A))
            .await
            .unwrap();
    }
    assert_eq!(
        EgressAudit::start(
            &scoped_request("POST", "https://x.example/", ORG_A),
            PolicyDecision::OpenOrg
        )
        .policy,
        "open_org"
    );
}

#[tokio::test]
async fn an_open_grant_does_not_lift_the_deny_list_or_reach_other_orgs() {
    let resolver = open_org_resolver();
    let service = curated_writes_service(10).with_org_allowlist(resolver.clone());
    let denied = scoped_request("POST", "https://x.webhook.site/collect", ORG_A);
    assert_eq!(
        service.decide_policy(&denied).await,
        PolicyDecision::Denied(EgressPolicyDenial::Denylisted)
    );
    assert_eq!(lookups(&resolver), 0, "deny list decides first");
    let other = scoped_request("POST", "https://hooks.customer.example/in", ORG_B);
    assert!(service.admit(&other).await.is_err());
    let unscoped = EgressRequest::new(
        "POST",
        "https://hooks.customer.example/in",
        EgressRequestKind::Capability,
    );
    assert!(service.admit(&unscoped).await.is_err());
}

#[tokio::test]
async fn runtime_services_use_the_installed_resolver() {
    let resolver = org_extension_resolver(&["api.runtime-installed.example"]);
    let mut service = curated_writes_service(10);
    service.org_allowlist = OrgAllowlistSource::Runtime;
    let request = scoped_request("POST", "https://api.runtime-installed.example/", ORG_A);
    crate::host::install_runtime_org_egress_allowlist(resolver);
    assert_eq!(
        service.decide_policy(&request).await,
        PolicyDecision::AllowlistedOrg
    );
    // A bare transport ignores the process-wide resolver.
    let bare = curated_writes_service(10);
    assert_eq!(
        bare.decide_policy(&request).await,
        PolicyDecision::Denied(EgressPolicyDenial::NotAllowlisted)
    );
}
