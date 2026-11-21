use axum::http::StatusCode;
use everruns_core::DeploymentGrade;
use everruns_server::records::{FeatureFlagGrade, FeatureFlagPolicy};
use serde_json::{Value, json};

use crate::test_harness::TestServer;

const FLAGS: &str = "/v1/orgs/org_00000000000000000000000000000001/feature-flags";

#[tokio::test]
async fn test_agent_view_settings_require_adoption_and_keep_saved_revisions() {
    let policy = FeatureFlagPolicy::from_env(DeploymentGrade::Prod)
        .with_grade("agent_views", FeatureFlagGrade::Adoption);
    let server = TestServer::in_memory_with_feature_policy(policy).await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({"name":"view-agent","system_prompt":"You help."}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let route = format!("/v1/agents/{}/view", agent["id"].as_str().unwrap());
    let view = json!({
        "title":"Changes",
        "layout":{"format":"html","source":"<h2>Changes</h2>"},
        "allow_personalization":true,
        "data_fields":["checked"],
        "allow_agent_layout":false
    });
    let request = json!({"revision":0, "view":view});

    server
        .get(&route)
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .put(&route, &request)
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .patch(FLAGS, json!({"flags":{"agent_views":true}}))
        .await
        .assert_status(StatusCode::OK);

    let initial: Value = server
        .get(&route)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(initial, json!({"revision":0,"view":null}));
    let saved: Value = server
        .put(&route, &request)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(saved, json!({"revision":1,"view":view}));
    let chat: Value = server
        .post(
            "/v1/sessions",
            json!({"harness_id":server.seed_base_harness_id}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let chat_view = format!(
        "/v1/sessions/{}/agent-view?view_agent={}",
        chat["id"].as_str().unwrap(),
        agent["id"].as_str().unwrap(),
    );
    let published: Value = server
        .get(&chat_view)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(published["title"], "Changes");
    assert_eq!(published["personal_layout"], Value::Null);
    let personal = json!({"format":"html","source":"<p>Mine</p>"});
    let customized: Value = server
        .put(&chat_view, json!({"revision":1, "layout":personal}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(customized["personal_layout"], personal);
    server
        .put(&route, &request)
        .await
        .assert_status(StatusCode::CONFLICT);
    server
        .put(&route, json!({"revision":1,"view":{"title":"Empty","layout":{"format":"a2ui","operations":[]}}}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    server
        .patch(FLAGS, json!({"flags":{"agent_views":false}}))
        .await
        .assert_status(StatusCode::OK);
    server
        .get(&route)
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .get(&chat_view)
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .patch(FLAGS, json!({"flags":{"agent_views":true}}))
        .await
        .assert_status(StatusCode::OK);
    let restored: Value = server
        .get(&route)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(restored, saved);
    let cleared: Value = server
        .put(&route, json!({"revision":1,"view":null}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(cleared, json!({"revision":2,"view":null}));
}

#[tokio::test]
async fn test_feature_grades_control_http_defaults_mutations_and_route_access() {
    for (grade, default_enabled, tenant_can_manage) in [
        (FeatureFlagGrade::Dev, false, false),
        (FeatureFlagGrade::Internal, false, false),
        (FeatureFlagGrade::Adoption, false, true),
        (FeatureFlagGrade::Prod, true, true),
        (FeatureFlagGrade::Off, false, false),
    ] {
        let policy = FeatureFlagPolicy::from_env(DeploymentGrade::Prod).with_grade("skills", grade);
        let server = TestServer::in_memory_with_feature_policy(policy).await;
        let effective: Value = server.get(FLAGS).await.assert_status(StatusCode::OK).json();
        assert_eq!(effective["skills"], default_enabled, "{grade}");

        for enabled in [true, false] {
            let response = server
                .patch(FLAGS, json!({"flags": {"skills": enabled}}))
                .await;
            if tenant_can_manage {
                let effective: Value = response.assert_status(StatusCode::OK).json();
                assert_eq!(effective["skills"], enabled);
                assert_eq!(
                    server
                        .db
                        .list_org_feature_flags(everruns_core::DEFAULT_ORG_ID)
                        .await
                        .unwrap()
                        .get("skills"),
                    Some(&enabled)
                );
            } else {
                response.assert_status(StatusCode::BAD_REQUEST);
            }
        }

        // The skills config route checks the org-effective flag on each request.
        server
            .get("/v1/skills/config")
            .await
            .assert_status(StatusCode::NOT_FOUND);

        let platform_route = format!("{FLAGS}/platform");
        let status = if matches!(
            grade,
            FeatureFlagGrade::Internal | FeatureFlagGrade::Adoption
        ) {
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        };
        server
            .patch(&platform_route, json!({"flags":{"skills":true}}))
            .await
            .assert_status(status);
        if matches!(
            grade,
            FeatureFlagGrade::Internal | FeatureFlagGrade::Adoption
        ) {
            let effective: Value = server.get(FLAGS).await.assert_status(StatusCode::OK).json();
            assert_eq!(effective["skills"], true);
            server
                .get("/v1/skills/config")
                .await
                .assert_status(StatusCode::OK);
            server
                .patch(&platform_route, json!({"flags":{"skills":false}}))
                .await
                .assert_status(StatusCode::OK);
            server
                .get("/v1/skills/config")
                .await
                .assert_status(StatusCode::NOT_FOUND);
        }
    }
}
