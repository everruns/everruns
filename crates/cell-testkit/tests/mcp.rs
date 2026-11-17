//! An org's MCP credentials on the data plane (data plane v2, step 11.2):
//! the control plane sends a server's header credential and the agent's
//! OAuth grant as sealed secrets; an executor opens them per call for the
//! calling session's org, and one executor at a time refreshes a grant, its
//! write reaching every executor and the control plane's feed.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use everruns::hosting::services::{McpAuthProvider, McpAuthRequest};
use everruns_cell::dp::{ConfigAck, ConfigChange, ConfigUpdate};
use everruns_cell::mcp::{Bindings, MCP_OAUTH, OrgMcp, mcp_binding_secret};
use everruns_cell::node_control::ControlPlane;
use everruns_cell::providers::OrgMaterial;
use everruns_cell::secrets::{SecretOp, SecretReply};
use everruns_cell_testkit::fake_cp::FakeCp;
use everruns_contracts::dataplane::{Connection, RefreshOwner};
use everruns_contracts::dataplane_seal as seal;
use everruns_contracts::typed_id::SessionId;
use reqwest::{Method, StatusCode};
use serde_json::json;
use support::Fleet;

const ORG: &str = "org-a";
const BINDING: &str = "MCP_DOCS_AUTHORIZATION";
const GRANT: &str = "mcp_oauth:grant-1";

fn tokens(access: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "access_token": access,
        "refresh_token": "refresh-0",
        "token_type": "Bearer",
        "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        "token_endpoint": "https://auth.example/token",
        "resource": "https://crm.example/mcp",
        "client_id": "client-1",
    }))
    .unwrap()
}

fn request(server: &str, session: SessionId) -> McpAuthRequest<'_> {
    McpAuthRequest {
        server_url: Some("https://mcp.example/mcp"),
        server_name: server,
        auth_mode: everruns_core::McpServerAuthMode::None,
        oauth_provider_id: None,
        session_id: Some(session.uuid()),
    }
}

fn bindings() -> Bindings {
    let mut bindings = Bindings::default();
    bindings.headers.insert(
        "docs".into(),
        vec![("Authorization".into(), BINDING.into())],
    );
    bindings.oauth.insert("crm".into(), GRANT.into());
    bindings
}

fn pct(mut values: Vec<Duration>, p: usize) -> Duration {
    values.sort();
    values[(values.len() * p / 100).min(values.len() - 1)]
}

#[tokio::test(flavor = "multi_thread")]
async fn an_orgs_mcp_credentials_open_on_its_executors_and_one_refreshes_a_grant() {
    let cp = FakeCp::start().await.unwrap();
    cp.set_balance(ORG, 10_000_000);
    let mut fleet = Fleet::start_with_cp(Some(ControlPlane::new(cp.url.clone()))).await;
    let base = fleet.base();

    let dp_key = seal::random_key();
    let data_key = seal::random_key();
    let wrapped =
        seal::wrap_data_key(&dp_key, ORG, "dk-1", &data_key, seal::random_nonce()).unwrap();
    let (status, ack) = cp
        .call(
            &base,
            Method::PUT,
            &format!("/dp/v1/orgs/{ORG}"),
            &json!({ "command_id": "enable-1", "wrapped_data_key": wrapped, "data_key_id": "dk-1" }),
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{ack}");

    let sealed = |id: &str, revision: u64, plain: &[u8]| {
        seal::seal(
            &data_key,
            "dk-1",
            ORG,
            id,
            revision,
            plain,
            seal::random_nonce(),
        )
        .unwrap()
    };
    let binding = mcp_binding_secret(BINDING);
    let mut package = everruns_cell_testkit::definition("echo").unwrap();
    package["name"] = json!("helper");
    let ack = cp
        .push_config(
            &base,
            ORG,
            &ConfigUpdate {
                epoch: 1,
                version: 1,
                base_version: 0,
                changes: vec![
                    ConfigChange::Secret(sealed(&binding, 1, b"Bearer docs-key")).to_value(),
                    ConfigChange::Secret(sealed(GRANT, 1, &tokens("access-0"))).to_value(),
                    ConfigChange::Connection(Connection {
                        connection_id: GRANT.into(),
                        kind: MCP_OAUTH.into(),
                        settings: json!({}),
                        token_secret_id: GRANT.into(),
                        refresh_owner: RefreshOwner::DataPlane,
                    })
                    .to_value(),
                    json!({ "kind": "Agent", "agent_id": "helper", "revision": 1, "package": package }),
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });
    let e1 = fleet.executor(1);
    e1.set_dp_key(dp_key);

    // A session of the org runs a turn on e1, which then knows its org.
    let id = SessionId::new();
    let (status, created) = cp
        .call(
            &base,
            Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": id, "agent_ref": "helper" }),
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let session = id.to_string();
    fleet.say(&session, "ping").await;
    fleet.turns(&session, 1).await;

    let auth = e1.mcp.for_agent(bindings());
    let docs = auth
        .authorization(&request("docs", id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(docs.authorization.as_deref(), Some("Bearer docs-key"));
    let crm = auth
        .authorization(&request("crm", id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(crm.authorization.as_deref(), Some("Bearer access-0"));
    // A session the executor does not know is no org's: nothing.
    let stranger = auth
        .authorization(&request("docs", SessionId::new()))
        .await
        .unwrap();
    assert!(stranger.is_none());

    // What a call pays for its credentials: opened from the directory cell
    // (a fresh executor's first call), and kept (every later call).
    let material: Arc<dyn OrgMaterial> = e1.clone();
    let mut cold = Vec::new();
    for _ in 0..50 {
        let fresh = Arc::new(OrgMcp::from_env());
        fresh.bind(Arc::downgrade(&material));
        let auth = fresh.for_agent(bindings());
        let started = Instant::now();
        auth.authorization(&request("docs", id))
            .await
            .unwrap()
            .unwrap();
        auth.authorization(&request("crm", id))
            .await
            .unwrap()
            .unwrap();
        cold.push(started.elapsed());
    }
    let mut warm = Vec::new();
    for _ in 0..500 {
        let started = Instant::now();
        auth.authorization(&request("docs", id))
            .await
            .unwrap()
            .unwrap();
        auth.authorization(&request("crm", id))
            .await
            .unwrap()
            .unwrap();
        warm.push(started.elapsed());
    }
    println!(
        "MCP credentials per call (a header and a grant): opened p50 {:?} p99 {:?}, kept p50 {:?} p99 {:?}",
        pct(cold.clone(), 50),
        pct(cold, 99),
        pct(warm.clone(), 50),
        pct(warm, 99),
    );

    // One refresher at a time, across executors, through the directory cell.
    let e2 = fleet.executor(2);
    e2.set_dp_key(dp_key);
    let lease = |holder: &str| SecretOp::Lease {
        secret_id: GRANT.into(),
        holder: holder.into(),
        revision: 1,
    };
    assert!(matches!(
        e1.secret_op(ORG, lease("e1")).await.unwrap(),
        SecretReply::Leased { .. }
    ));
    assert!(matches!(
        e2.secret_op(ORG, lease("e2")).await.unwrap(),
        SecretReply::Held { .. }
    ));
    let opened = e1.open_secret(ORG, GRANT).await.unwrap().unwrap();
    let next = seal::seal(
        &opened.data_key,
        &opened.data_key_id,
        ORG,
        GRANT,
        opened.revision + 1,
        &tokens("access-1"),
        seal::random_nonce(),
    )
    .unwrap();
    // Only the holder writes.
    let refreshed = |holder: &str| SecretOp::Refreshed {
        holder: holder.into(),
        secret: next.clone(),
    };
    assert_eq!(
        e2.secret_op(ORG, refreshed("e2")).await.unwrap(),
        SecretReply::Changed { revision: 1 }
    );
    assert_eq!(
        e1.secret_op(ORG, refreshed("e1")).await.unwrap(),
        SecretReply::Stored { revision: 2 }
    );
    let read = e2.open_secret(ORG, GRANT).await.unwrap().unwrap();
    assert_eq!(read.revision, 2);
    assert_eq!(read.plain, tokens_access(&read.plain, "access-1"));
    // The control plane hears of it on the feed.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !cp.refreshed().contains(&(ORG.into(), GRANT.into(), 2)) {
        assert!(Instant::now() < deadline, "{:?}", cp.refreshed());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `plain` when its access token is `access`; anything else otherwise.
fn tokens_access(plain: &[u8], access: &str) -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_slice(plain).unwrap();
    if value["access_token"] == access {
        plain.to_vec()
    } else {
        Vec::new()
    }
}
