//! The rest of the control plane contract against the cells (data plane v2
//! step 7): the link's token, org tokens at the gateway, an org's data key
//! and sealed secrets, suspension, export and feed replay.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::time::{Duration, Instant};

use everruns_cell::dp::{ConfigAck, ConfigChange, ConfigUpdate};
use everruns_cell::node_control::ControlPlane;
use everruns_cell_testkit::fake_cp::FakeCp;
use everruns_contracts::dataplane_seal as seal;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use support::Fleet;

const ORG: &str = "org-a";
const LINK: &str = "link-secret";

fn agent(id: &str) -> Value {
    let mut package = everruns_cell_testkit::definition("worker").unwrap();
    package["name"] = json!(id);
    json!({ "kind": "Agent", "agent_id": id, "revision": 1, "package": package })
}

async fn linked(org_tokens: bool) -> (Fleet, FakeCp) {
    let cp = FakeCp::start().await.unwrap();
    cp.set_link_token(LINK);
    cp.set_balance(ORG, 10_000_000);
    let mut link = ControlPlane::new(cp.url.clone());
    link.token = Some(LINK.into());
    link.jwks_url = org_tokens.then(|| cp.jwks_url());
    let mut fleet = Fleet::start_with_cp(Some(link)).await;
    fleet.executor(1);
    let update = ConfigUpdate {
        epoch: 1,
        version: 1,
        base_version: 0,
        changes: vec![agent("helper")],
    };
    let ack = cp.push_config(&fleet.base(), ORG, &update).await.unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 1 });
    (fleet, cp)
}

/// `method` `path` on the gateway with `token` as bearer.
async fn as_caller(
    fleet: &Fleet,
    token: Option<&str>,
    method: Method,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = fleet
        .http
        .request(method, format!("{}{path}", fleet.base()))
        .json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let reply = request.send().await.unwrap();
    let status = reply.status();
    let text = reply.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(json!(text)))
}

async fn wait_turns(fleet: &Fleet, token: &str, session: &str, turns: u64) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, view) = as_caller(
            fleet,
            Some(token),
            Method::GET,
            &format!("/v1/sessions/{session}/wait?turns={turns}&timeout_ms=5000"),
            Value::Null,
        )
        .await;
        if view["turns_completed"].as_u64() >= Some(turns) {
            return;
        }
        assert!(Instant::now() < deadline, "{session}: {view}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_link_and_the_gateway_take_only_tokens_they_trust() {
    let (fleet, cp) = linked(true).await;
    // The link: config without the link's token is refused.
    let (status, _) = as_caller(
        &fleet,
        None,
        Method::PUT,
        &format!("/dp/v1/orgs/{ORG}/config/1/2"),
        json!({ "epoch": 1, "version": 2, "base_version": 1, "changes": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = as_caller(
        &fleet,
        Some("wrong"),
        Method::POST,
        "/dp/v1/feed/replay",
        json!({ "command_id": "r", "source": "cell-0", "incarnation": 1, "from_index": 0 }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Org tokens: none, a forged one, another org's.
    let create = json!({ "org": ORG, "agent_ref": "helper" });
    let mine = cp.org_token(ORG, "sessions:write").unwrap();
    let theirs = cp.org_token("org-b", "sessions:write").unwrap();
    let forged = everruns_cell_testkit::signer::Signer::new("cp-key-1")
        .unwrap()
        .token(ORG, "dp-local", "admin", 600)
        .unwrap();
    let elsewhere = cp
        .signer
        .token(ORG, "dp-other", "sessions:write", 600)
        .unwrap();
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(forged.as_str()), StatusCode::UNAUTHORIZED),
        (Some(elsewhere.as_str()), StatusCode::UNAUTHORIZED),
        (Some(theirs.as_str()), StatusCode::FORBIDDEN),
    ] {
        let (status, body) =
            as_caller(&fleet, token, Method::POST, "/v1/sessions", create.clone()).await;
        assert_eq!(status, expected, "{body}");
    }

    // Mine: the session is the org's, and only the org reaches it.
    let (status, created) =
        as_caller(&fleet, Some(&mine), Method::POST, "/v1/sessions", create).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let session = created["id"].as_str().unwrap().to_owned();
    let (status, _) = as_caller(
        &fleet,
        Some(&theirs),
        Method::GET,
        &format!("/v1/sessions/{session}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another org's session");
    let (status, _) = as_caller(
        &fleet,
        Some(&mine),
        Method::POST,
        &format!("/v1/sessions/{session}/messages"),
        json!({ "text": "go" }),
    )
    .await;
    assert!(status.is_success(), "{status}");
    wait_turns(&fleet, &mine, &session, 1).await;

    // Fleet operations need admin.
    let (status, _) = as_caller(&fleet, Some(&mine), Method::GET, "/v1/cells", Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let admin = cp.org_token(ORG, "admin").unwrap();
    let (status, _) = as_caller(&fleet, Some(&admin), Method::GET, "/v1/cells", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_open_only_on_executors_and_suspension_stops_new_turns() {
    let (mut fleet, cp) = linked(false).await;
    let base = fleet.base();
    // Enable the org with its data key, wrapped under the data plane's key.
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
    assert_eq!(ack["accepted"], true);
    let sealed = seal::seal(
        &data_key,
        "dk-1",
        ORG,
        "github",
        1,
        b"ghp_token",
        seal::random_nonce(),
    )
    .unwrap();
    let ack = cp
        .push_config(
            &base,
            ORG,
            &ConfigUpdate {
                epoch: 1,
                version: 2,
                base_version: 1,
                changes: vec![ConfigChange::Secret(sealed.clone()).to_value()],
            },
        )
        .await
        .unwrap();
    assert_eq!(ack, ConfigAck::Ok { applied_version: 2 });
    let executor = fleet.executor(2);
    assert!(executor.secret(ORG, "github").await.is_err(), "no key yet");
    executor.set_dp_key(dp_key);
    assert_eq!(executor.secret(ORG, "github").await.unwrap(), b"ghp_token");
    assert!(executor.secret(ORG, "missing").await.is_err());

    // Suspension: new turns wait, the org event reaches the feed; resume.
    let session = {
        let created = fleet
            .post("/v1/sessions", json!({ "org": ORG, "agent_ref": "helper" }))
            .await;
        created["id"].as_str().unwrap().to_owned()
    };
    fleet.say(&session, "one").await;
    fleet.turns(&session, 1).await;
    let suspend = json!({ "command_id": "s-1", "state": "suspended", "reason": "billing" });
    let (_, ack) = cp
        .call(
            &base,
            Method::POST,
            &format!("/dp/v1/orgs/{ORG}/state"),
            &suspend,
        )
        .await
        .unwrap();
    assert_eq!(ack["accepted"], true, "{ack}");
    // Every leader and executor hears within a poll.
    tokio::time::sleep(everruns_cell::tenants::POLL * 3).await;
    fleet.say(&session, "two").await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    let view = fleet.get(&format!("/v1/sessions/{session}")).await;
    assert_eq!(
        view["turns_completed"], 1,
        "a suspended org started a turn: {view}"
    );
    fleet
        .until("the suspension reaches the feed", || async {
            cp.org_events()
                .contains(&(ORG.to_owned(), "suspended".to_owned()))
        })
        .await;
    let resume = json!({ "command_id": "s-2", "state": "active" });
    cp.call(
        &base,
        Method::POST,
        &format!("/dp/v1/orgs/{ORG}/state"),
        &resume,
    )
    .await
    .unwrap();
    fleet.turns(&session, 2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_export_lands_in_the_store_and_a_replay_is_stored_once() {
    let (fleet, cp) = linked(false).await;
    let base = fleet.base();
    let session = fleet
        .post("/v1/sessions", json!({ "org": ORG, "agent_ref": "helper" }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;

    let (_, ack) = cp
        .call(
            &base,
            Method::POST,
            &format!("/dp/v1/orgs/{ORG}/exports"),
            &json!({ "command_id": "x-1", "export_id": "e-1" }),
        )
        .await
        .unwrap();
    assert_eq!(ack["accepted"], true, "{ack}");
    fleet
        .until("the export is done", || async {
            let (_, status) = cp
                .call(
                    &base,
                    Method::GET,
                    &format!("/dp/v1/orgs/{ORG}/commands/x-1"),
                    &Value::Null,
                )
                .await
                .unwrap();
            assert_ne!(status["state"], "failed", "{status}");
            status["state"] == "done"
        })
        .await;
    let compressed = fleet
        .store()
        .get_object(&format!("orgs/{ORG}/exports/e-1.tar.zst"))
        .await
        .unwrap();
    let tar = zstd_decode(&compressed);
    let names = tar_names(&tar);
    assert!(names.contains(&"config.json".to_owned()), "{names:?}");
    assert!(
        names.contains(&format!("sessions/{session}.json")),
        "{names:?}"
    );

    // Replay the whole archived feed: the control plane stores nothing twice.
    fleet
        .until("the feed is delivered and archived", || async {
            let mut pending = 0;
            for cell in [0, 1] {
                pending += fleet.stats(cell).await["feed_pending"]
                    .as_u64()
                    .unwrap_or(0);
            }
            pending == 0 && !cp.usage().is_empty()
        })
        .await;
    let usage = cp.usage();
    let mut resent = 0;
    for source in ["cell-0", "cell-1"] {
        let (status, reply) = cp
            .call(
                &base,
                Method::POST,
                "/dp/v1/feed/replay",
                &json!({ "command_id": format!("r-{source}"), "source": source, "incarnation": 1, "from_index": 0 }),
            )
            .await
            .unwrap();
        assert_eq!(status, StatusCode::OK, "{reply}");
        resent += reply["resent"].as_u64().unwrap();
    }
    assert!(resent > 0, "nothing was archived");
    assert_eq!(
        cp.duplicates() as u64,
        resent,
        "every replayed record was a repeat"
    );
    assert_eq!(cp.usage(), usage, "usage is stored once");
}

fn zstd_decode(bytes: &[u8]) -> Vec<u8> {
    zstd::decode_all(bytes).unwrap()
}

fn tar_names(tar: &[u8]) -> Vec<String> {
    let mut archive = tar::Archive::new(tar);
    archive
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().display().to_string())
        .collect()
}
