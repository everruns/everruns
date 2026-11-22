//! An org enabled on a fleet, as the control plane enables one (data plane
//! v2, step 11.5): its sessions and the session commands the control plane
//! sends.

use everruns_agentd::dp::{ConfigAck, ConfigChange, ConfigUpdate};
use everruns_agentd::node_control::ControlPlane;
use everruns_agentd_testkit::fake_cp::FakeCp;
use everruns_contracts::dataplane::TenantPolicy;
use everruns_contracts::dataplane_seal as seal;
use everruns_contracts::typed_id::SessionId;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::Fleet;

pub const ORG: &str = "org-a";
/// `ORG`'s data key, as the control plane keeps it.
pub const DATA_KEY: seal::Key = [7; 32];
/// Its id.
pub const DATA_KEY_ID: &str = "dk-1";

/// A fleet with `ORG` enabled, running the `echo` agent as `helper`, its
/// policy tapping `events`; and an executor.
pub async fn org_fleet(events: &[&str]) -> (Fleet, FakeCp) {
    let cp = FakeCp::start().await.unwrap();
    cp.set_balance(ORG, 1_000_000_000);
    let mut fleet = Fleet::start_with_cp(Some(ControlPlane::new(cp.url.clone()))).await;
    let base = fleet.base();
    let dp_key = seal::random_key();
    let wrapped =
        seal::wrap_data_key(&dp_key, ORG, DATA_KEY_ID, &DATA_KEY, seal::random_nonce()).unwrap();
    let (status, ack) = cp
        .call(
            &base,
            Method::PUT,
            &format!("/dp/v1/orgs/{ORG}"),
            &json!({ "command_id": "enable-1", "wrapped_data_key": wrapped, "data_key_id": DATA_KEY_ID }),
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{ack}");
    let mut package = everruns_agentd_testkit::definition("echo").unwrap();
    package["name"] = json!("helper");
    let policy = TenantPolicy {
        events: events.iter().map(|t| t.to_string()).collect(),
        ..TenantPolicy::default()
    };
    let ack = cp
        .push_config(
            &base,
            ORG,
            &ConfigUpdate {
                epoch: 1,
                version: 1,
                base_version: 0,
                changes: vec![
                    ConfigChange::Policy(policy).to_value(),
                    json!({ "kind": "Agent", "agent_id": "helper", "revision": 1, "package": package }),
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });
    fleet.executor(1).set_dp_key(dp_key);
    // Every shard's leader reads the org's tapped types with the tenants
    // view, once a poll.
    tokio::time::sleep(everruns_agentd::tenants::POLL * 2).await;
    (fleet, cp)
}

/// The executor [`org_fleet`] started.
pub fn org_executor(fleet: &Fleet) -> std::sync::Arc<everruns_agentd::executor::Executor> {
    fleet.executors[&1].0.clone()
}

/// A session of `ORG`, created as the control plane does.
pub async fn session(fleet: &Fleet, cp: &FakeCp) -> String {
    let id = SessionId::new();
    let (status, created) = cp
        .call(
            &fleet.base(),
            Method::POST,
            &format!("/dp/v1/orgs/{ORG}/sessions"),
            &json!({ "id": id, "agent_ref": "helper" }),
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED, "{created}");
    id.to_string()
}

/// One of the session commands a channel sends; its status and answer.
pub async fn command(
    fleet: &Fleet,
    cp: &FakeCp,
    org: &str,
    session: &str,
    command: &str,
    body: Value,
) -> (StatusCode, Value) {
    cp.call(
        &fleet.base(),
        Method::POST,
        &format!("/dp/v1/orgs/{org}/sessions/{session}/{command}"),
        &body,
    )
    .await
    .unwrap()
}
