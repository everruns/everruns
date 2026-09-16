//! Per-endpoint publish and the agent-level exposure suspend (EVE-1007).
//!
//! The point of this change is **sibling isolation**: before it, obtaining a
//! Slack manifest required publishing the whole App, which simultaneously
//! exposed every other endpoint attached to it — including an anonymous public
//! chat surface. These tests pin that publishing one endpoint moves exactly one
//! endpoint, and that the two agent-level terms behave as documented:
//!
//! ```text
//! live(endpoint) = endpoint.status == live
//!               && agent.status == active
//!               && !agent.exposures_suspended
//! ```
//!
//! The agent-level terms are enforced at resolution time, never by rewriting
//! endpoint rows, so the tests assert the stored status is untouched.
//!
//! Run with: cargo test -p everruns-server --test endpoint_publish_test -- --test-threads=1

mod test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

/// An app with two sibling endpoints on one agent: an `api_endpoint` and an
/// `a2a` channel. Both start enabled and unpublished.
struct Siblings {
    agent_id: String,
    app_id: String,
    api_channel_id: String,
    a2a_channel_id: String,
}

async fn create_siblings(server: &TestServer, label: &str) -> Siblings {
    // The test database persists between runs, and agent names are unique per
    // org, so every fixture needs its own name.
    let name = &format!("{label}-{}", uuid::Uuid::new_v4().simple());
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("{name}-agent"),
                "display_name": format!("{name} agent"),
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap().to_string();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": name,
                "harness_id": server.seed_generic_harness_id.clone(),
                "agent_id": agent["id"],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let app_id = app["id"].as_str().unwrap().to_string();

    let api_channel: Value = server
        .post(
            &format!("/v1/apps/{app_id}/api-endpoint-channels"),
            json!({ "session_mode": "shared_session" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let api_channel_id = channel_id_of(&api_channel);

    let a2a_channel: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels"),
            json!({
                "session_mode": "shared_session",
                "message": "Handle this A2A request",
                "agent_card_name": "Sibling endpoint",
                "agent_card_description": "Endpoint used to prove sibling isolation",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let a2a_channel_id = channel_id_of(&a2a_channel);

    Siblings {
        agent_id,
        app_id,
        api_channel_id,
        a2a_channel_id,
    }
}

/// Both channel-create responses wrap the channel alongside generated secrets,
/// so dig out the channel id whichever shape came back.
fn channel_id_of(response: &Value) -> String {
    for key in ["id", "channel_id"] {
        if let Some(id) = response.get(key).and_then(Value::as_str) {
            return id.to_string();
        }
    }
    for key in ["channel", "app_channel"] {
        if let Some(id) = response
            .get(key)
            .and_then(|c| c.get("id"))
            .and_then(Value::as_str)
        {
            return id.to_string();
        }
    }
    panic!("could not find a channel id in {response}");
}

async fn channel_status(server: &TestServer, app_id: &str, channel_id: &str) -> String {
    let app: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let channels = app["channels"].as_array().expect("channels array");
    let channel = channels
        .iter()
        .find(|c| c["id"].as_str() == Some(channel_id))
        .unwrap_or_else(|| panic!("channel {channel_id} not on app {app_id}"));
    channel["status"]
        .as_str()
        .unwrap_or_else(|| panic!("channel {channel_id} has no status: {channel}"))
        .to_string()
}

async fn agent_card_status(server: &TestServer, channel_id: &str) -> StatusCode {
    server
        .get(&format!(
            "/v1/e/{channel_id}/a2a/.well-known/agent-card.json"
        ))
        .await
        .status()
}

/// The headline: publishing one endpoint must not make its sibling reachable.
#[tokio::test]
async fn publishing_one_endpoint_does_not_expose_its_sibling() {
    let server = TestServer::new().await;
    let s = create_siblings(&server, "sibling-isolation").await;

    // Nothing published yet: the A2A agent card is not served.
    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::NOT_FOUND,
        "an unpublished endpoint must not serve its agent card"
    );

    // Publish only the api_endpoint sibling.
    server
        .post(
            &format!(
                "/v1/apps/{}/channels/{}/publish",
                s.app_id, s.api_channel_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        channel_status(&server, &s.app_id, &s.api_channel_id).await,
        "live"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.a2a_channel_id).await,
        "draft",
        "publishing one endpoint must leave its sibling's status alone"
    );
    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::NOT_FOUND,
        "publishing the api_endpoint sibling must not expose the A2A endpoint"
    );

    // And the reverse direction: publishing A2A does not disturb the sibling.
    server
        .post(
            &format!(
                "/v1/apps/{}/channels/{}/publish",
                s.app_id, s.a2a_channel_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::OK,
        "a published A2A endpoint serves its agent card"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.api_channel_id).await,
        "live",
        "publishing a sibling must not disturb the already-live endpoint"
    );
}

/// The incident control: one switch takes every endpoint off the internet, and
/// clearing it restores exactly the set that was live — without the endpoint
/// rows ever being rewritten.
#[tokio::test]
async fn suspending_exposures_refuses_traffic_and_resuming_restores_the_live_set() {
    let server = TestServer::new().await;
    let s = create_siblings(&server, "suspend-restore").await;

    // One endpoint live, one deliberately left in draft.
    server
        .post(
            &format!(
                "/v1/apps/{}/channels/{}/publish",
                s.app_id, s.a2a_channel_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::OK
    );

    let agent: Value = server
        .post(
            &format!("/v1/agents/{}/exposures/suspend", s.agent_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(agent["exposures_suspended"], json!(true));
    assert_eq!(
        agent["exposed"],
        json!(false),
        "a suspended agent is not exposed, whatever its endpoints say"
    );

    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::NOT_FOUND,
        "suspending exposures must refuse traffic on every endpoint"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.a2a_channel_id).await,
        "live",
        "suspend must not rewrite endpoint status — that is what makes resume exact"
    );

    let agent: Value = server
        .post(
            &format!("/v1/agents/{}/exposures/resume", s.agent_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(agent["exposures_suspended"], json!(false));
    assert_eq!(agent["exposed"], json!(true));

    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::OK,
        "resuming restores the previously live endpoint"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.api_channel_id).await,
        "draft",
        "resume must not raise an endpoint that was never live"
    );
}

/// An archived agent has no reachable endpoint, enforced at resolution time —
/// so un-archiving restores the previous live set with no row rewritten.
#[tokio::test]
async fn archived_agent_has_no_reachable_endpoint_and_unarchiving_restores_it() {
    let server = TestServer::new().await;
    let s = create_siblings(&server, "archived-agent").await;

    server
        .post(
            &format!(
                "/v1/apps/{}/channels/{}/publish",
                s.app_id, s.a2a_channel_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::OK
    );

    // Archive travels through the agent update as a status change.
    server
        .patch(
            &format!("/v1/agents/{}", s.agent_id),
            json!({ "status": "archived" }),
        )
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::NOT_FOUND,
        "an archived agent must have no reachable endpoint"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.a2a_channel_id).await,
        "live",
        "archiving must not rewrite endpoint rows"
    );

    // There is no un-archive transition on the API today — an archived agent
    // cannot be edited — so restore the agent directly to prove the property
    // that matters: because archiving rewrote no endpoint rows, making the
    // agent active again is enough to bring back exactly the live set.
    sqlx::query("UPDATE agents SET status = 'active' WHERE public_id = $1")
        .bind(&s.agent_id)
        .execute(&server.pool)
        .await
        .expect("restore agent");

    assert_eq!(
        agent_card_status(&server, &s.a2a_channel_id).await,
        StatusCode::OK,
        "restoring the agent restores exactly the endpoints that were live"
    );
}

/// Derived agent exposure state must track the endpoint rows rather than being
/// stored, in every combination.
#[tokio::test]
async fn derived_exposure_matches_the_endpoint_rows() {
    let server = TestServer::new().await;
    let s = create_siblings(&server, "derived-exposure").await;

    let agent: Value = server
        .get(&format!("/v1/agents/{}", s.agent_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        agent["exposed"],
        json!(false),
        "no live endpoint means not exposed"
    );

    server
        .post(
            &format!(
                "/v1/apps/{}/channels/{}/publish",
                s.app_id, s.a2a_channel_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);

    let agent: Value = server
        .get(&format!("/v1/agents/{}", s.agent_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        agent["exposed"],
        json!(true),
        "one live endpoint means exposed"
    );

    server
        .post(
            &format!(
                "/v1/apps/{}/channels/{}/unpublish",
                s.app_id, s.a2a_channel_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);

    let agent: Value = server
        .get(&format!("/v1/agents/{}", s.agent_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        agent["exposed"],
        json!(false),
        "unpublishing the last live endpoint clears the derived flag"
    );
}

/// The App-level publish switch still works while the App domain exists: it
/// moves the endpoints it owns, and leaves a disabled one disabled.
#[tokio::test]
async fn app_publish_still_drives_its_endpoints() {
    let server = TestServer::new().await;
    let s = create_siblings(&server, "app-publish-bridge").await;

    // Disable one endpoint before publishing the App.
    server
        .patch(
            &format!("/v1/apps/{}/channels/{}", s.app_id, s.api_channel_id),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);

    server
        .post(&format!("/v1/apps/{}/publish", s.app_id), json!({}))
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        channel_status(&server, &s.app_id, &s.a2a_channel_id).await,
        "live",
        "publishing the App raises the endpoints the operator left enabled"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.api_channel_id).await,
        "disabled",
        "an explicitly disabled endpoint stays disabled across an App publish"
    );

    server
        .post(&format!("/v1/apps/{}/unpublish", s.app_id), json!({}))
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        channel_status(&server, &s.app_id, &s.a2a_channel_id).await,
        "draft"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.api_channel_id).await,
        "disabled",
        "unpublishing must not resurrect a disabled endpoint as draft"
    );
}

/// TM-AUTHZ-006 / TM-TENANT-002: every not-live reason must be indistinguishable
/// from "no such endpoint". A caller must not be able to tell a draft endpoint
/// from a suspended agent from a channel id that was never issued.
#[tokio::test]
async fn non_live_rejections_are_indistinguishable_from_not_found() {
    let server = TestServer::new().await;
    let s = create_siblings(&server, "generic-rejection").await;

    let unknown = "appchan_00000000000000000000000000000000";
    let never_issued = agent_card_status(&server, unknown).await;
    let draft_endpoint = agent_card_status(&server, &s.a2a_channel_id).await;

    server
        .post(
            &format!(
                "/v1/apps/{}/channels/{}/publish",
                s.app_id, s.a2a_channel_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);
    server
        .post(
            &format!("/v1/agents/{}/exposures/suspend", s.agent_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);
    let suspended_agent = agent_card_status(&server, &s.a2a_channel_id).await;

    assert_eq!(never_issued, StatusCode::NOT_FOUND);
    assert_eq!(
        draft_endpoint, never_issued,
        "a draft endpoint must look exactly like one that was never issued"
    );
    assert_eq!(
        suspended_agent, never_issued,
        "a suspended agent must look exactly like an endpoint that was never issued"
    );
}

async fn assert_channel_added_to_published_app_starts_draft(server: TestServer, label: &str) {
    let s = create_siblings(&server, label).await;

    server
        .post(&format!("/v1/apps/{}/publish", s.app_id), json!({}))
        .await
        .assert_status(StatusCode::OK);

    let added: Value = server
        .post(
            &format!("/v1/apps/{}/a2a-channels", s.app_id),
            json!({
                "session_mode": "shared_session",
                "message": "Handle this A2A request",
                "agent_card_name": "Added after publish",
                "agent_card_description": "Endpoint created while the App was already published",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let added_id = channel_id_of(&added);

    assert_eq!(
        channel_status(&server, &s.app_id, &added_id).await,
        "draft",
        "a new endpoint must require an explicit publish"
    );
    assert_eq!(
        agent_card_status(&server, &added_id).await,
        StatusCode::NOT_FOUND,
        "a new endpoint must not be reachable before it is published"
    );
    assert_eq!(
        channel_status(&server, &s.app_id, &s.a2a_channel_id).await,
        "live",
        "creating an endpoint must not change existing endpoint rows"
    );

    server
        .post(&format!("/v1/apps/{}/unpublish", s.app_id), json!({}))
        .await
        .assert_status(StatusCode::OK);
    server
        .post(&format!("/v1/apps/{}/publish", s.app_id), json!({}))
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        channel_status(&server, &s.app_id, &added_id).await,
        "live",
        "publishing the App must still raise its enabled endpoints"
    );
    assert_eq!(
        agent_card_status(&server, &added_id).await,
        StatusCode::OK,
        "the endpoint must become reachable after the App publishes it"
    );
}

#[tokio::test]
async fn channel_added_to_published_app_starts_draft_postgres() {
    assert_channel_added_to_published_app_starts_draft(TestServer::new().await, "draft-pg").await;
}

#[tokio::test]
async fn channel_added_to_published_app_starts_draft_in_memory() {
    assert_channel_added_to_published_app_starts_draft(
        TestServer::in_memory().await,
        "draft-memory",
    )
    .await;
}
