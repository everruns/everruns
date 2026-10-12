use super::*;
use crate::domains::agents::commands::CreateAgent;
use crate::domains::agents::types::CreateAgentRequest;
use crate::kernel_imports::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
use crate::services::CapabilityService;
use base64::engine::general_purpose::STANDARD;
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;
use uuid::Uuid;

async fn owner_ctx() -> Ctx {
    let db = Arc::new(StorageBackend::test_database());
    crate::setup::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
    Ctx::new(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(Uuid::nil()),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        },
        db,
        capability_service,
        None,
        Arc::new(crate::kernel_imports::DefaultPermissionResolver),
    )
}

async fn create_agent(ctx: &Ctx, name: &str) -> String {
    let request: CreateAgentRequest = serde_json::from_value(serde_json::json!({
        "name": name,
        "system_prompt": "initial prompt",
    }))
    .expect("agent request");
    CreateAgent(request)
        .execute(ctx)
        .await
        .expect("create agent")
        .public_id
        .to_string()
}

fn png(edge: u32) -> Vec<u8> {
    let image = RgbaImage::from_pixel(edge, edge, Rgba([20, 120, 220, 255]));
    let mut buffer = Cursor::new(Vec::new());
    image.write_to(&mut buffer, ImageFormat::Png).unwrap();
    buffer.into_inner()
}

trait On {
    fn on(self, agent_id: &str) -> SetAgentAvatar;
}

impl On for SetAgentAvatarRequest {
    fn on(self, agent_id: &str) -> SetAgentAvatar {
        SetAgentAvatar {
            agent_id: agent_id.to_string(),
            req: self,
        }
    }
}

async fn stored_source(ctx: &Ctx, agent_id: &str) -> Option<String> {
    let row = ctx
        .db
        .get_agent_by_public_id(ctx.org_id(), agent_id)
        .await
        .unwrap()
        .unwrap();
    ctx.db
        .get_agent_avatar_source(ctx.org_id(), row.id.uuid())
        .await
        .unwrap()
}

fn assert_bad_request(err: &CommandError, needle: &str) {
    assert_eq!(err.status().as_u16(), 400, "message: {}", err.message());
    assert!(
        err.message().contains(needle),
        "expected {needle:?} in {:?}",
        err.message()
    );
}

#[tokio::test]
async fn sets_a_curated_preset() {
    let ctx = owner_ctx().await;
    let agent_id = create_agent(&ctx, "preset-agent").await;
    let preset = PRESETS[0].metadata.id;

    let avatar = SetAgentAvatarRequest {
        preset_id: Some(preset.to_string()),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .expect("preset avatar");

    assert!(avatar.url.ends_with("/square-256.png"), "{}", avatar.url);
    assert_eq!(
        stored_source(&ctx, &agent_id).await.as_deref(),
        Some(format!("preset:{preset}").as_str())
    );
    let variant = ctx
        .db
        .get_agent_avatar_variant(avatar.id.uuid(), "square-512.png")
        .await
        .unwrap()
        .expect("rendered variant stored");
    assert_eq!(variant.content_type, "image/png");
}

#[tokio::test]
async fn sets_an_inline_image_and_replaces_the_previous_avatar() {
    let ctx = owner_ctx().await;
    let agent_id = create_agent(&ctx, "upload-agent").await;
    let first = SetAgentAvatarRequest {
        preset_id: Some(PRESETS[0].metadata.id.to_string()),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .unwrap();

    // Line-wrapped like `base64` prints it, with the type detected from the bytes.
    let encoded = STANDARD.encode(png(64));
    let wrapped: Vec<String> = encoded
        .as_bytes()
        .chunks(76)
        .map(|line| String::from_utf8(line.to_vec()).unwrap())
        .collect();
    let avatar = SetAgentAvatarRequest {
        image_base64: Some(wrapped.join("\n")),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .expect("inline avatar");

    assert_ne!(avatar.id, first.id, "a new image gets a new id");
    assert_eq!(
        stored_source(&ctx, &agent_id).await.as_deref(),
        Some("upload")
    );
    assert!(
        ctx.db
            .get_agent_avatar_variant(first.id.uuid(), "square-256.png")
            .await
            .unwrap()
            .is_none(),
        "the replaced avatar stops resolving"
    );

    // A data URI carries its own type.
    SetAgentAvatarRequest {
        image_base64: Some(format!("data:image/png;base64,{encoded}")),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .expect("data URI avatar");
}

#[tokio::test]
async fn rejects_bad_inline_images() {
    let ctx = owner_ctx().await;
    let agent_id = create_agent(&ctx, "bad-image-agent").await;
    let run = |image_base64: String, content_type: Option<&str>| {
        SetAgentAvatarRequest {
            image_base64: Some(image_base64),
            content_type: content_type.map(str::to_string),
            ..Default::default()
        }
        .on(&agent_id)
    };

    let err = run("not base64!!".into(), None)
        .execute(&ctx)
        .await
        .unwrap_err();
    assert_bad_request(&err, "not valid base64");

    let oversized = STANDARD.encode(vec![0u8; MAX_INLINE_AVATAR_BYTES + 1]);
    let err = run(oversized, None).execute(&ctx).await.unwrap_err();
    assert_bad_request(&err, "at most 1 MB");

    let err = run(STANDARD.encode(b"plain text"), None)
        .execute(&ctx)
        .await
        .unwrap_err();
    assert_bad_request(&err, "pass content_type");

    let err = run(STANDARD.encode(png(64)), Some("image/jpeg"))
        .execute(&ctx)
        .await
        .unwrap_err();
    assert_bad_request(&err, "could not be read");

    let err = run(STANDARD.encode(png(32)), None)
        .execute(&ctx)
        .await
        .unwrap_err();
    assert_bad_request(&err, "at least 64x64");

    assert_eq!(stored_source(&ctx, &agent_id).await, None, "nothing stored");
}

#[tokio::test]
async fn requires_exactly_one_source() {
    let ctx = owner_ctx().await;
    let agent_id = create_agent(&ctx, "one-source-agent").await;

    let err = SetAgentAvatarRequest::default()
        .on(&agent_id)
        .execute(&ctx)
        .await
        .unwrap_err();
    assert_bad_request(&err, "exactly one of preset_id or image_base64");

    let err = SetAgentAvatarRequest {
        preset_id: Some(PRESETS[0].metadata.id.to_string()),
        image_base64: Some(STANDARD.encode(png(64))),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .unwrap_err();
    assert_bad_request(&err, "exactly one of preset_id or image_base64");

    let err = SetAgentAvatarRequest {
        preset_id: Some(PRESETS[0].metadata.id.to_string()),
        content_type: Some("image/png".into()),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .unwrap_err();
    assert_bad_request(&err, "content_type applies to image_base64 only");
}

#[tokio::test]
async fn rejects_unknown_preset_and_unknown_agent() {
    let ctx = owner_ctx().await;
    let agent_id = create_agent(&ctx, "unknown-preset-agent").await;

    let err = SetAgentAvatarRequest {
        preset_id: Some("no-such-preset".into()),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .unwrap_err();
    assert_bad_request(&err, "Unknown avatar preset");

    let missing = AgentId::from_uuid(Uuid::now_v7()).to_string();
    let err = SetAgentAvatarRequest {
        preset_id: Some(PRESETS[0].metadata.id.to_string()),
        ..Default::default()
    }
    .on(&missing)
    .execute(&ctx)
    .await
    .unwrap_err();
    assert_eq!(err.status().as_u16(), 404, "{}", err.message());

    let err = SetAgentAvatarRequest {
        preset_id: Some(PRESETS[0].metadata.id.to_string()),
        ..Default::default()
    }
    .on("unknown-preset-agent")
    .execute(&ctx)
    .await
    .unwrap_err();
    assert_bad_request(&err, "agent_id must be an agent ID");
}

#[tokio::test]
async fn built_in_agent_rejects_set_agent_avatar() {
    let ctx = owner_ctx().await;
    let agent_id = create_agent(&ctx, "chat").await;
    ctx.db
        .mark_agent_built_in(DEFAULT_ORG_ID, agent_id.parse().unwrap())
        .await
        .unwrap();

    let err = SetAgentAvatarRequest {
        preset_id: Some(PRESETS[0].metadata.id.to_string()),
        ..Default::default()
    }
    .on(&agent_id)
    .execute(&ctx)
    .await
    .unwrap_err();
    assert_bad_request(&err, "built-in agent");
    assert!(err.message().contains("Copy it first"), "{}", err.message());
    assert_eq!(stored_source(&ctx, &agent_id).await, None);
}

#[tokio::test]
async fn set_agent_avatar_records_history_through_run() {
    let ctx = owner_ctx().await;
    let agent_id = create_agent(&ctx, "history-agent").await;

    SetAgentAvatarRequest {
        preset_id: Some(PRESETS[0].metadata.id.to_string()),
        ..Default::default()
    }
    .on(&agent_id)
    .run(&ctx)
    .await
    .expect("run");

    let changes = ctx
        .db
        .list_entity_changes(&crate::storage::entity_changes::EntityChangeQuery {
            org_id: ctx.org_id(),
            entity_ref: Some(agent_id.clone()),
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    let entry = changes
        .iter()
        .find(|c| c.command == "set_agent_avatar")
        .expect("history entry");
    assert_eq!(entry.action, "updated");
    assert_eq!(entry.changed_fields, vec!["preset_id".to_string()]);
}

#[tokio::test]
async fn lists_the_curated_presets() {
    let ctx = owner_ctx().await;
    let presets = ListAvatarPresets {}.run(&ctx).await.expect("list presets");
    assert_eq!(presets.len(), PRESETS.len());
    assert!(presets.iter().all(|p| find_preset(p.id).is_some()));
}
