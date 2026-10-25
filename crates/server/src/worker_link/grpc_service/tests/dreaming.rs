use super::*;

fn has_dreaming(configs: &[String]) -> bool {
    configs.iter().any(|value| {
        serde_json::from_str::<everruns_contracts::CapabilityRef>(value)
            .unwrap()
            .capability_id()
            == "dreaming"
    })
}

#[tokio::test]
async fn standalone_runtime_reads_obey_dreaming_adoption_and_revocation() {
    let service = test_worker_service().await;
    let org = everruns_core::DEFAULT_ORG_ID;
    service
        .db
        .replace_org_feature_flags(
            org,
            &std::collections::HashMap::from([("dreaming".to_string(), true)]),
        )
        .await
        .unwrap();
    let config = serde_json::json!({"ref":"dreaming","config":{"destinations":[{"mount":"/memory/user","path":"/learned"}]}});
    let agent = execute_test_command(&service, "create_agent", serde_json::json!({
        "name":"dreaming-rpc-test", "system_prompt":"Test", "harness_name":"worker", "capabilities":[config.clone()]
    })).await;
    let agent_id: everruns_contracts::typed_id::AgentId =
        agent["id"].as_str().unwrap().parse().unwrap();
    let harness = service
        .db
        .get_harness_by_name(org, "worker")
        .await
        .unwrap()
        .unwrap();
    let session = execute_test_command(&service, "create_session", serde_json::json!({
        "agent_id":agent_id, "harness_id":harness.id, "tags":[], "capabilities":[config], "tools":[], "mcp_servers":{}, "initial_files":[]
    })).await;
    let session_id: everruns_contracts::typed_id::SessionId =
        session["id"].as_str().unwrap().parse().unwrap();
    for enabled in [false, true, false] {
        service
            .db
            .replace_org_feature_flags(
                org,
                &std::collections::HashMap::from([("dreaming".to_string(), enabled)]),
            )
            .await
            .unwrap();
        let agent = service
            .get_agent(Request::new(GetAgentRequest {
                org_id: org,
                agent_id: Some(proto::Uuid {
                    value: agent_id.uuid().to_string(),
                }),
            }))
            .await
            .unwrap()
            .into_inner()
            .agent
            .unwrap();
        assert_eq!(
            agent.capability_ids.iter().any(|id| id == "dreaming"),
            enabled
        );
        assert_eq!(has_dreaming(&agent.capabilities), enabled);
        let harness = service
            .get_harness(Request::new(GetHarnessRequest {
                org_id: org,
                harness_id: Some(proto::Uuid {
                    value: harness.id.uuid().to_string(),
                }),
            }))
            .await
            .unwrap()
            .into_inner()
            .harness
            .unwrap();
        assert_eq!(
            harness.capability_ids.iter().any(|id| id == "dreaming"),
            enabled
        );
        assert_eq!(has_dreaming(&harness.capabilities), enabled);
        let session = service
            .get_session(Request::new(GetSessionRequest {
                org_id: org,
                session_id: Some(proto::Uuid {
                    value: session_id.uuid().to_string(),
                }),
            }))
            .await
            .unwrap()
            .into_inner()
            .session
            .unwrap();
        assert_eq!(has_dreaming(&session.capabilities), enabled);
    }
}
