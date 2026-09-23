use super::*;
use crate::domains::agent_channels::types::{CreateAgentChannelRequest, UpdateAgentChannelRequest};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateAgentRow, CreateHarnessRow};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use everruns_platform::{ChannelStatus, ChannelType};
use everruns_provider::typed_id::AgentId;
use serde_json::json;
use std::sync::Arc;

async fn seed_agent(db: &StorageBackend) -> String {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "channel-harness".to_string(),
                display_name: Some("Channel Harness".to_string()),
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
            public_id: public_id.clone(),
            name: "channel-agent".to_string(),
            display_name: Some("Channel Agent".to_string()),
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
async fn channel_commands_cover_the_management_lifecycle() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let created = CreateAgentChannel {
        agent_id: agent_id.clone(),
        req: CreateAgentChannelRequest {
            channel_type: ChannelType::Webhook,
            channel_config: json!({
                "token": "channel-secret",
                "message": "Process {{payload}}",
            }),
            enabled: true,
        },
    }
    .run(&ctx)
    .await
    .expect("create channel");
    assert_eq!(created.status, ChannelStatus::Draft);
    assert!(created.enabled);

    let channel_id = created.public_id.to_string();
    let listed = ListAgentChannels {
        agent_id: agent_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("list channels");
    assert_eq!(listed.len(), 1);

    let disabled = UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
        req: UpdateAgentChannelRequest {
            channel_config: None,
            enabled: Some(false),
        },
    }
    .run(&ctx)
    .await
    .expect("disable channel");
    assert_eq!(disabled.status, ChannelStatus::Disabled);
    assert!(!disabled.enabled);

    let enabled = UpdateAgentChannelCmd {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
        req: UpdateAgentChannelRequest {
            channel_config: None,
            enabled: Some(true),
        },
    }
    .run(&ctx)
    .await
    .expect("enable channel");
    assert_eq!(enabled.status, ChannelStatus::Draft);

    let published = PublishAgentChannel {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("publish channel");
    assert_eq!(published.status, ChannelStatus::Live);

    let unpublished = UnpublishAgentChannel {
        agent_id: agent_id.clone(),
        channel_id: channel_id.clone(),
    }
    .run(&ctx)
    .await
    .expect("unpublish channel");
    assert_eq!(unpublished.status, ChannelStatus::Draft);

    DeleteAgentChannel {
        agent_id: agent_id.clone(),
        channel_id,
    }
    .run(&ctx)
    .await
    .expect("delete channel");
    let listed = ListAgentChannels { agent_id }
        .run(&ctx)
        .await
        .expect("list channels after delete");
    assert!(listed.is_empty());
}

#[tokio::test]
async fn native_schedule_creation_uses_agent_triggers_instead() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent_id = seed_agent(&db).await;
    let ctx = test_ctx(db);

    let error = CreateAgentChannel {
        agent_id,
        req: CreateAgentChannelRequest {
            channel_type: ChannelType::Schedule,
            channel_config: json!({
                "cron_expression": "0 0 * * * * *",
                "timezone": "UTC",
                "session_mode": "channel",
                "message": "Run",
            }),
            enabled: true,
        },
    }
    .run(&ctx)
    .await
    .expect_err("schedule channel creation must use agent triggers");

    assert!(matches!(
        error.kind,
        CommandErrorKind::BadRequest(message)
            if message == "Create schedules through agent triggers"
    ));
}
