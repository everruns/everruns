use axum::http::StatusCode;
use everruns_core::DeploymentGrade;
use everruns_server::records::{FeatureFlagGrade, FeatureFlagPolicy};
use serde_json::{Value, json};

use crate::test_harness::TestServer;

const FLAGS: &str = "/v1/orgs/org_00000000000000000000000000000001/feature-flags";

#[tokio::test]
async fn test_agent_view_settings_are_always_available_and_keep_saved_revisions() {
    let policy = FeatureFlagPolicy::from_env(DeploymentGrade::Prod);
    let server = TestServer::in_memory_with_feature_policy(policy).await;
    let agent: Value = server
        .post(
            "/v1/platform/agents",
            json!({"name":"view-agent","system_prompt":"You help."}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let route = format!("/v1/platform/agents/{}/view", agent["id"].as_str().unwrap());
    let view = json!({
        "title":"Changes",
        "layout":{"format":"html","source":"<h2>Changes</h2>"},
        "allow_personalization":true,
        "data_fields":["checked"],
        "allow_agent_layout":false
    });
    let request = json!({"revision":0, "view":view});

    // A retired false override must not disable a graduated feature.
    server
        .db
        .replace_org_feature_flags(
            everruns_core::DEFAULT_ORG_ID,
            &std::collections::HashMap::from([("agent_views".into(), false)]),
        )
        .await
        .unwrap();
    let flags: Value = server.get(FLAGS).await.assert_status(StatusCode::OK).json();
    assert!(flags.get("agent_views").is_none());
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
            "/v1/platform/sessions",
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
        let policy = FeatureFlagPolicy::from_env(DeploymentGrade::Prod).with_grade("evals", grade);
        let server = TestServer::in_memory_with_feature_policy(policy).await;
        // The general API fixture opts into Evals. This test measures grade
        // defaults before exercising explicit tenant and platform overrides.
        sqlx::query("DELETE FROM org_feature_flags WHERE org_id=$1 AND flag_name='evals'")
            .bind(everruns_core::DEFAULT_ORG_ID)
            .execute(&server.pool)
            .await
            .unwrap();
        let effective: Value = server.get(FLAGS).await.assert_status(StatusCode::OK).json();
        assert_eq!(effective["evals"], default_enabled, "{grade}");

        for enabled in [true, false] {
            let response = server
                .patch(FLAGS, json!({"flags": {"evals": enabled}}))
                .await;
            if tenant_can_manage {
                let effective: Value = response.assert_status(StatusCode::OK).json();
                assert_eq!(effective["evals"], enabled);
                assert_eq!(
                    server
                        .db
                        .list_org_feature_flags(everruns_core::DEFAULT_ORG_ID)
                        .await
                        .unwrap()
                        .get("evals"),
                    Some(&enabled)
                );
            } else {
                response.assert_status(StatusCode::BAD_REQUEST);
            }
        }

        // The Evals route still follows its org-effective rollout flag.
        let view_route = "/v1/platform/evals";
        server
            .get(view_route)
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
            .patch(&platform_route, json!({"flags":{"evals":true}}))
            .await
            .assert_status(status);
        if matches!(
            grade,
            FeatureFlagGrade::Internal | FeatureFlagGrade::Adoption
        ) {
            let effective: Value = server.get(FLAGS).await.assert_status(StatusCode::OK).json();
            assert_eq!(effective["evals"], true);
            server.get(view_route).await.assert_status(StatusCode::OK);
            server
                .patch(&platform_route, json!({"flags":{"evals":false}}))
                .await
                .assert_status(StatusCode::OK);
            server
                .get(view_route)
                .await
                .assert_status(StatusCode::NOT_FOUND);
        }
    }
}
