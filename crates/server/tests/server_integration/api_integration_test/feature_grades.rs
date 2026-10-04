use axum::http::StatusCode;
use everruns_core::DeploymentGrade;
use everruns_platform::{FeatureFlagGrade, FeatureFlagPolicy};
use serde_json::{Value, json};

use crate::test_harness::TestServer;

const FLAGS: &str = "/v1/orgs/org_00000000000000000000000000000001/feature-flags";

#[tokio::test]
async fn test_feature_grades_control_http_defaults_mutations_and_payment_access() {
    for (grade, default_enabled, tenant_can_manage) in [
        (FeatureFlagGrade::Dev, false, false),
        (FeatureFlagGrade::Internal, false, false),
        (FeatureFlagGrade::Adoption, false, true),
        (FeatureFlagGrade::Prod, true, true),
        (FeatureFlagGrade::Off, false, false),
    ] {
        let policy = FeatureFlagPolicy::from_env(DeploymentGrade::Prod)
            .with_grade("machine_payments", grade);
        let server = TestServer::in_memory_with_feature_policy(policy).await;
        let effective: Value = server.get(FLAGS).await.assert_status(StatusCode::OK).json();
        assert_eq!(effective["machine_payments"], default_enabled, "{grade}");

        for enabled in [true, false] {
            let response = server
                .patch(FLAGS, json!({"flags": {"machine_payments": enabled}}))
                .await;
            if tenant_can_manage {
                let effective: Value = response.assert_status(StatusCode::OK).json();
                assert_eq!(effective["machine_payments"], enabled);
                assert_eq!(
                    server
                        .db
                        .list_org_feature_flags(everruns_core::DEFAULT_ORG_ID)
                        .await
                        .unwrap()
                        .get("machine_payments"),
                    Some(&enabled)
                );
            } else {
                response.assert_status(StatusCode::BAD_REQUEST);
            }
        }

        // No payment setup or spend occurs: this exercises the real query gate.
        server
            .get("/v1/payments/accounts")
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
            .patch(&platform_route, json!({"flags":{"machine_payments":true}}))
            .await
            .assert_status(status);
        if matches!(
            grade,
            FeatureFlagGrade::Internal | FeatureFlagGrade::Adoption
        ) {
            let effective: Value = server.get(FLAGS).await.assert_status(StatusCode::OK).json();
            assert_eq!(effective["machine_payments"], true);
            server
                .get("/v1/payments/accounts")
                .await
                .assert_status(StatusCode::OK);
            server
                .patch(&platform_route, json!({"flags":{"machine_payments":false}}))
                .await
                .assert_status(StatusCode::OK);
            server
                .get("/v1/payments/accounts")
                .await
                .assert_status(StatusCode::NOT_FOUND);
        }
    }
}
