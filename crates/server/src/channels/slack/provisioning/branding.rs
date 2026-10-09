use super::{SlackProvisioningResult, malformed_response};
use crate::channels::slack::events::{
    build_agent_description, build_long_description, build_short_description, truncate_chars,
    truncate_display_name,
};

pub(super) fn patch(
    manifest: &mut serde_json::Value,
    name: &str,
    description: Option<&str>,
) -> SlackProvisioningResult<()> {
    // Slack replaces the whole manifest. Refuse malformed parents rather than losing
    // configuration; change only identity fields and retain prompts, scopes and URLs.
    for pointer in [
        "/display_information",
        "/features",
        "/features/bot_user",
        "/features/agent_view",
        "/features/assistant_view",
    ] {
        if manifest
            .pointer(pointer)
            .is_some_and(|value| !value.is_object())
        {
            return Err(malformed_response("apps.manifest.export"));
        }
    }
    manifest["display_information"]["name"] = truncate_chars(name, 35).into();
    manifest["display_information"]["description"] =
        build_short_description(name, description).into();
    manifest["display_information"]["long_description"] =
        build_long_description(name, description).into();
    if let Some(bot) = manifest.pointer_mut("/features/bot_user") {
        bot["display_name"] = truncate_display_name(name).into();
    }
    // Existing agent surfaces may still use Slack's legacy spelling. Never enable
    // a surface or migrate its spelling as a side effect of changing identity.
    for surface in ["agent_view", "assistant_view"] {
        if let Some(view) = manifest.pointer_mut(&format!("/features/{surface}")) {
            let field = if surface == "agent_view" {
                "agent_description"
            } else {
                "assistant_description"
            };
            view[field] = build_agent_description(name, description).into();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{provisioner, stored};
    use super::*;
    use crate::domains::agent_channels::record::slack_provisioning::SlackAppProvisioner;
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn branding_update_preserves_settings_and_updates_enabled_agent_view() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        let manifest = json!({
            "display_information":{"name":"Old", "description":"Old", "long_description":"Old", "background_color":"#123456"},
            "features":{"bot_user":{"display_name":"Old", "always_online":false}, "agent_view":{"agent_description":"Old", "suggested_prompts":[{"title":"Custom", "message":"Keep me"}]}},
            "settings":{"event_subscriptions":{"request_url":"https://example.test/events"}},
            "oauth_config":{"redirect_urls":["https://example.test/callback"],"scopes":{"bot":["chat:write","custom:read"]}},
            "_metadata":{"major_version":2}
        });
        Mock::given(method("POST"))
            .and(path("/apps.manifest.export"))
            .and(header("authorization", "Bearer access"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok":true,"manifest":manifest})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apps.manifest.update"))
            .and(header("authorization", "Bearer access"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .expect(1)
            .mount(&server)
            .await;
        provisioner
            .update_branding(1, Some("T1"), "A1", "New name", Some("New description"))
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(body["app_id"], "A1");
        let updated: Value = serde_json::from_str(body["manifest"].as_str().unwrap()).unwrap();
        assert_eq!(updated["display_information"]["name"], "New name");
        assert_eq!(
            updated["display_information"]["description"],
            "New description"
        );
        assert!(
            updated["display_information"]["long_description"]
                .as_str()
                .unwrap()
                .contains("New description")
        );
        assert_eq!(updated["features"]["bot_user"]["display_name"], "New name");
        assert_eq!(
            updated["features"]["agent_view"]["agent_description"],
            "New name — New description"
        );
        let mut expected = manifest;
        expected["display_information"] = updated["display_information"].clone();
        expected["features"]["bot_user"]["display_name"] = json!("New name");
        expected["features"]["agent_view"]["agent_description"] =
            json!("New name — New description");
        assert_eq!(updated, expected);
        assert_eq!(
            updated["display_information"]["background_color"],
            "#123456"
        );
    }
    #[test]
    fn branding_patch_preserves_disabled_and_legacy_agent_surfaces_and_bounds_unicode() {
        let name = "名".repeat(90);
        let description = "描".repeat(5000);
        for surface in [None, Some("assistant_view")] {
            let mut manifest = json!({"display_information":{},"features":{"bot_user":{}}});
            if let Some(surface) = surface {
                manifest["features"][surface] =
                    json!({"assistant_description":"Old", "suggested_prompts":[{"title":"Keep"}]});
            }
            patch(&mut manifest, &name, Some(&description)).unwrap();
            assert_eq!(
                manifest["display_information"]["name"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .count(),
                35
            );
            assert_eq!(
                manifest["display_information"]["description"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .count(),
                140
            );
            assert_eq!(
                manifest["display_information"]["long_description"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .count(),
                4000
            );
            assert!(manifest["features"].get("agent_view").is_none());
            if surface.is_some() {
                assert_eq!(
                    manifest["features"]["assistant_view"]["assistant_description"]
                        .as_str()
                        .unwrap()
                        .chars()
                        .count(),
                    300
                );
                assert_eq!(
                    manifest["features"]["assistant_view"]["suggested_prompts"],
                    json!([{"title":"Keep"}])
                );
            }
        }
        let mut manifest = json!({});
        patch(&mut manifest, "Bot", Some("  ")).unwrap();
        assert_eq!(
            manifest["display_information"]["description"],
            "Bot (Powered by Everruns)"
        );
        assert!(
            manifest.get("features").is_none(),
            "identity updates never enable a removed bot feature"
        );
        assert!(
            (174..=4000).contains(
                &build_long_description(&"名".repeat(40), None)
                    .chars()
                    .count()
            )
        );
    }

    #[tokio::test]
    async fn branding_update_refuses_malformed_exports_and_wrong_workspace() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        assert!(
            provisioner
                .update_branding(2, Some("T1"), "A1", "Bot", None)
                .await
                .is_err()
        );
        assert!(
            provisioner
                .update_branding(1, Some("T2"), "A1", "Bot", None)
                .await
                .is_err()
        );
        Mock::given(path("/apps.manifest.export"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"manifest":{"features":[]}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/apps.manifest.update"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        assert!(
            provisioner
                .update_branding(1, Some("T1"), "A1", "Bot", None)
                .await
                .is_err()
        );
        for pointer in ["display_information", "features"] {
            let mut manifest = json!({});
            manifest[pointer] = json!("invalid");
            assert!(patch(&mut manifest, "Bot", None).is_err());
        }
        for field in ["bot_user", "agent_view", "assistant_view"] {
            let mut manifest = json!({"features":{}});
            manifest["features"][field] = json!("invalid");
            assert!(patch(&mut manifest, "Bot", None).is_err());
        }
    }
    #[tokio::test]
    async fn branding_rest_edit_reaches_slack_without_reinstalling() {
        use crate::{
            api, auth,
            domains::{
                agents::{CreateAgent, types::CreateAgentRequest},
                common::{Command, Ctx},
            },
            services::CapabilityService,
            storage::CreateAgentChannelRow,
        };
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use everruns_core::{Caller, DEFAULT_ORG_ID, DeploymentGrade};
        use std::sync::Arc;
        use tower::ServiceExt;
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(DEFAULT_ORG_ID, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        crate::setup::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
            .await
            .unwrap();
        let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None);
        let agent = CreateAgent(
            serde_json::from_value::<CreateAgentRequest>(
                json!({"name":"rest-branding", "system_prompt":"Be helpful"}),
            )
            .unwrap(),
        )
        .run(&ctx)
        .await
        .unwrap();
        db.create_agent_channel(DEFAULT_ORG_ID, CreateAgentChannelRow {
            agent_id: agent.internal_id, public_id: format!("appchan_{}", uuid::Uuid::now_v7().simple()), channel_type: "slack".into(),
            channel_config: json!({"provisioned_app":{"app_id":"A1","client_id":"c1","client_secret":"s1","team_id":"T1"}}),
            channel_config_encrypted: None, auth: None, auth_encrypted: None, enabled: true, status: "live".into(), virtual_user_id: None, owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1).uuid(), resolved_owner_user_id: None,
        }).await.unwrap();
        let manifest = json!({"display_information":{"name":"Old"}, "features":{"bot_user":{"display_name":"Old"}}, "oauth_config":{"scopes":{"bot":["chat:write"]}}});
        Mock::given(path("/apps.manifest.export"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok":true,"manifest":manifest})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/apps.manifest.update"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .expect(1)
            .mount(&server)
            .await;
        let auth = auth::AuthState::builtin(auth::AuthConfig::default(), db.clone());
        let state = api::agents::AppState::new(
            db.clone(),
            None,
            Arc::new(CapabilityService::new(db, None)),
            auth,
            DeploymentGrade::Dev,
            Arc::new(crate::platform::oss_host_composition()),
            Arc::new(crate::platform::oss_built_in_harnesses()),
        )
        .with_slack_provisioner(Some(Arc::new(provisioner)));
        let response = api::agents::routes(state).oneshot(Request::builder().method("PATCH").uri(format!("/v1/agents/{}",agent.public_id))
            .header("content-type","application/json").body(Body::from(json!({"display_name":"Renamed in Everruns","description":"Edited in Everruns"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let requests = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let requests = server.received_requests().await.unwrap();
                if requests.len() == 2 {
                    break requests;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("REST edit reaches Slack");
        let body: Value = serde_json::from_slice(&requests[1].body).unwrap();
        let updated: Value = serde_json::from_str(body["manifest"].as_str().unwrap()).unwrap();
        assert_eq!(
            updated["display_information"]["name"],
            "Renamed in Everruns"
        );
        assert_eq!(
            updated["display_information"]["description"],
            "Edited in Everruns"
        );
        assert_eq!(
            updated["features"]["bot_user"]["display_name"],
            "Renamed in Everruns"
        );
        assert_eq!(updated["oauth_config"], manifest["oauth_config"]);
    }

    #[tokio::test]
    async fn branding_export_http_rate_limit_is_retryable_without_a_json_body() {
        let server = MockServer::start().await;
        let (provisioner, db) = provisioner(&server);
        db.upsert_org_slack_connection(stored(1, "T1", chrono::Duration::hours(1)))
            .await
            .unwrap();
        Mock::given(path("/apps.manifest.export"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        assert!(
            matches!(provisioner.update_branding(1, Some("T1"), "A1", "Bot", None).await,
            Err(crate::domains::agent_channels::record::slack_provisioning::SlackProvisioningError::Rejected(code)) if code == "ratelimited")
        );
    }
    #[test]
    fn branding_creation_and_updates_render_the_same_identity() {
        use crate::channels::slack::events::build_manifest_yaml;
        let name = "A \"quoted\" 名\nline";
        let description = "Description with a newline\nand a \"quote\"";
        let yaml = build_manifest_yaml(
            name,
            &truncate_display_name(name),
            Some(description),
            "https://example.test/events",
            "https://example.test/actions",
            "https://example.test/callback",
            true,
            &[],
        );
        let mut manifest: Value = serde_yaml::from_str(&yaml).unwrap();
        let created = manifest.clone();
        patch(&mut manifest, name, Some(description)).unwrap();
        assert_eq!(manifest, created);
    }
}
