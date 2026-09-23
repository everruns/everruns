use super::*;
use crate::domains::agent_endpoints::types::{
    CreateAgentEndpointRequest, UpdateAgentEndpointRequest,
};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateAgentRow, CreateHarnessRow};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use everruns_platform::{ChannelType, EndpointStatus};
use everruns_provider::typed_id::AgentId;
use serde_json::json;
use std::sync::Arc;

async fn seed_agent(db: &StorageBackend) -> String {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "endpoint-harness".to_string(),
                display_name: Some("Endpoint Harness".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                embedder_metadata: json!({}),
                is_built_in: false,
            },
        )
        .await
        .expect("create harness");
    let public_id = AgentId::new().to_string();
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            project_id: everruns_core::DEFAULT_PROJECT_ID,
            public_id: public_id.clone(),
            name: "endpoint-agent".to_string(),
            display_name: Some("Endpoint Agent".to_string()),
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: json!([]),
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: harness.id,
            tags: vec![],
            initial_files: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            is_built_in: false,
        },
    )
    .await
    .expect("create agent");
    public_id
}

fn test_ctx(db: Arc<StorageBackend>) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
}

#[tokio::test]
async fn endpoint_commands_cover_the_management_lifecycle() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let created = CreateAgentEndpoint {
        agent_id: agent_id.clone(),
        req: CreateAgentEndpointRequest {
            channel_type: ChannelType::Webhook,
            channel_config: json!({
                "token": "endpoint-secret",
                "message": "Process {{payload}}",
            }),
            enabled: true,
        },
    }
    .run(&ctx)
    .await
    .expect("create endpoint");
    assert_eq!(created.status, EndpointStatus::Draft);
    assert!(created.enabled);

    let endpoint_id = created.public_id.to_string();
    let listed = ListAgentEndpoints {
        agent_id: agent_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("list endpoints");
    assert_eq!(listed.len(), 1);

    let disabled = UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
        req: UpdateAgentEndpointRequest {
            channel_config: None,
            enabled: Some(false),
        },
    }
    .run(&ctx)
    .await
    .expect("disable endpoint");
    assert_eq!(disabled.status, EndpointStatus::Disabled);
    assert!(!disabled.enabled);

    let enabled = UpdateAgentEndpointCmd {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
        req: UpdateAgentEndpointRequest {
            channel_config: None,
            enabled: Some(true),
        },
    }
    .run(&ctx)
    .await
    .expect("enable endpoint");
    assert_eq!(enabled.status, EndpointStatus::Draft);

    let published = PublishAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("publish endpoint");
    assert_eq!(published.status, EndpointStatus::Live);

    let unpublished = UnpublishAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id: endpoint_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("unpublish endpoint");
    assert_eq!(unpublished.status, EndpointStatus::Draft);

    DeleteAgentEndpoint {
        agent_id: agent_id.clone(),
        endpoint_id,
    }
    .run(&ctx)
    .await
    .expect("delete endpoint");
    let listed = ListAgentEndpoints { agent_id }
        .run(&ctx)
        .await
        .expect("list endpoints after delete");
    assert!(listed.is_empty());
}

#[tokio::test]
async fn native_schedule_creation_uses_agent_triggers_instead() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let error = CreateAgentEndpoint {
        agent_id,
        req: CreateAgentEndpointRequest {
            channel_type: ChannelType::Schedule,
            channel_config: json!({
                "cron_expression": "0 0 * * * * *",
                "timezone": "UTC",
                "session_mode": "endpoint",
                "message": "Run",
            }),
            enabled: true,
        },
    }
    .run(&ctx)
    .await
    .expect_err("schedule endpoint creation must use agent triggers");

    assert!(matches!(
        error.kind,
        CommandErrorKind::BadRequest(message)
            if message == "Create schedules through agent triggers"
    ));
}
