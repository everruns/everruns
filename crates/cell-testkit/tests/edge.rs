//! The edge router against a live fleet and a fake control plane (data
//! plane v2, step 8): one URL, a v2 org's session calls served by the data
//! plane with an org token the edge exchanged for the caller's own
//! credential, records and everything else passed to the control plane.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::Arc;
use std::time::Duration;

use everruns_cell::dp::{ConfigAck, ConfigUpdate};
use everruns_cell::edge::Edge;
use everruns_cell::gateway::Gateway;
use everruns_cell::node_control::ControlPlane;
use everruns_cell::org_token::OrgTokens;
use everruns_cell_testkit::fake_cp::FakeCp;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use support::Fleet;

const ORG: &str = "org-a";
const LINK: &str = "link-secret";

struct Setup {
    fleet: Fleet,
    cp: FakeCp,
    edge: String,
    /// A session of `ORG` on the data plane.
    session: String,
}

async fn setup() -> Setup {
    let cp = FakeCp::start().await.unwrap();
    cp.set_link_token(LINK);
    cp.set_balance(ORG, 10_000_000);
    let mut link = ControlPlane::new(cp.url.clone());
    link.token = Some(LINK.into());
    link.jwks_url = Some(cp.jwks_url());
    let mut fleet = Fleet::start_with_cp(Some(link)).await;
    fleet.executor(1);
    let mut package = everruns_cell_testkit::definition("echo").unwrap();
    package["name"] = json!("helper");
    let update = ConfigUpdate {
        epoch: 1,
        version: 1,
        base_version: 0,
        changes: vec![
            json!({ "kind": "Agent", "agent_id": "helper", "revision": 1, "package": package }),
        ],
    };
    let ack = cp.push_config(&fleet.base(), ORG, &update).await.unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });

    let tokens = Arc::new(OrgTokens::new(Some(cp.jwks_url()), Some("dp-local".into())));
    let edge = Edge::new(Gateway::new(fleet.topology()), &cp.url, tokens);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = edge.router();
    tokio::spawn(async move { axum::serve(listener, app).await });

    let token = cp.org_token(ORG, "sessions:write").unwrap();
    let created: Value = fleet
        .http
        .post(format!("{}/v1/sessions", fleet.base()))
        .bearer_auth(token)
        .json(&json!({ "org": ORG, "agent_ref": "helper" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session = created["id"].as_str().unwrap().to_owned();
    Setup {
        fleet,
        cp,
        edge: url,
        session,
    }
}

impl Setup {
    /// `method` `path` through the edge, with `headers`.
    async fn call(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .fleet
            .http
            .request(method, format!("{}{path}", self.edge));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let reply = request.send().await.unwrap();
        let status = reply.status();
        let text = reply.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(json!(text)))
    }

    async fn events(&self, credential: &str) -> (StatusCode, Value) {
        self.call(
            Method::GET,
            &format!("/v1/sessions/{}/events", self.session),
            &[("authorization", credential)],
            None,
        )
        .await
    }
}

fn from_control_plane(body: &Value) -> bool {
    body["served_by"] == "control-plane"
}

#[tokio::test(flavor = "multi_thread")]
async fn a_v2_orgs_session_calls_reach_the_data_plane_and_records_the_control_plane() {
    let s = setup().await;
    let me = [("authorization", "Bearer user-org-a")];
    let (status, sent) = s
        .call(
            Method::POST,
            &format!("/v1/sessions/{}/messages", s.session),
            &me,
            Some(json!({ "message": { "role": "user", "content": [{ "type": "text", "text": "hi" }] } })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");
    assert_eq!(sent["status"], "accepted");
    let (status, events) = s.events("Bearer user-org-a").await;
    assert_eq!(status, StatusCode::OK, "{events}");
    assert!(!from_control_plane(&events));
    assert!(events["data"].as_array().is_some_and(|e| !e.is_empty()));

    // The record and everything else: the control plane, with the
    // caller's own credential.
    for path in [
        format!("/v1/sessions/{}", s.session),
        format!("/v1/sessions/{}/pin", s.session),
        "/v1/sessions".to_owned(),
        "/v1/agents".to_owned(),
    ] {
        let (status, body) = s.call(Method::GET, &path, &me, None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(from_control_plane(&body), "{path}: {body}");
        assert_eq!(body["authorization"], "Bearer user-org-a");
    }
    // A stage 2 call is the data plane's, which says it does not serve it.
    let (status, body) = s
        .call(
            Method::GET,
            &format!("/v1/sessions/{}/fs/notes.md", s.session),
            &me,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    assert_eq!(s.cp.exchanges(), 1, "the exchanged token is reused");

    // An org that is not on a data plane, and a session the data plane
    // does not hold: the control plane.
    let (_, body) = s.events("Bearer v1-user").await;
    assert!(from_control_plane(&body), "{body}");
    let other = everruns_contracts::typed_id::SessionId::new();
    let (_, body) = s
        .call(
            Method::GET,
            &format!("/v1/sessions/{other}/events"),
            &me,
            None,
        )
        .await;
    assert!(from_control_plane(&body), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_token_reaches_another_orgs_session() {
    let s = setup().await;
    // A member of another org on the data plane.
    let (status, body) = s.events("Bearer user-org-b").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // An org token sent outright: expired, or signed by someone else.
    let expired =
        s.cp.signer
            .token(ORG, "dp-local", "sessions:write", -120)
            .unwrap();
    let forged = everruns_cell_testkit::signer::Signer::new("cp-key-1")
        .unwrap()
        .token(ORG, "dp-local", "admin", 600)
        .unwrap();
    for token in [expired, forged] {
        for header in ["authorization", "x-everruns-org-token"] {
            let value = if header == "authorization" {
                format!("Bearer {token}")
            } else {
                token.clone()
            };
            let (status, body) = s
                .call(
                    Method::GET,
                    &format!("/v1/sessions/{}/events", s.session),
                    &[(header, &value)],
                    None,
                )
                .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{header}: {body}");
        }
    }
    // A valid one works the same way as an exchanged one.
    let mine = s.cp.org_token(ORG, "sessions:write").unwrap();
    let exchanged = s.cp.exchanges();
    let (status, _) = s
        .call(
            Method::GET,
            &format!("/v1/sessions/{}/events", s.session),
            &[("cookie", &format!("everruns_org_token={mine}"))],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        s.cp.exchanges(),
        exchanged,
        "nothing exchanged for a token sent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exchanged_token_is_refreshed_before_it_expires() {
    let s = setup().await;
    s.cp.set_token_ttl_secs(4);
    let (status, _) = s.events("Bearer user-org-a").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(s.cp.exchanges(), 1);
    // Halfway through a short life, the next call fetches a fresh one.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let (status, _) = s.events("Bearer user-org-a").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(s.cp.exchanges(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_control_plane_down_a_session_keeps_working() {
    let s = setup().await;
    let (status, _) = s.events("Bearer user-org-a").await;
    assert_eq!(status, StatusCode::OK);
    s.cp.set_down(true);
    let (status, sent) = s
        .call(
            Method::POST,
            &format!("/v1/sessions/{}/messages", s.session),
            &[("authorization", "Bearer user-org-a")],
            Some(json!({ "text": "still here" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");
    let (status, _) = s.events("Bearer user-org-a").await;
    assert_eq!(status, StatusCode::OK);
    // The control plane's own calls wait for it.
    let (status, _) = s
        .call(
            Method::GET,
            "/v1/agents",
            &[("authorization", "Bearer user-org-a")],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
