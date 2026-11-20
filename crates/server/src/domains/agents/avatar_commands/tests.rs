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
            read_only: false,
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

// ============================================================================
// generate_agent_avatar
// ============================================================================

mod generate {
    use super::*;
    use crate::domains::providers::ProviderService;
    use crate::services::ProviderResolverService;
    use crate::storage::{CreateProviderRow, EncryptionService};
    use everruns_core::{
        EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    };
    use std::sync::Mutex;

    /// Stands in for the image provider behind the host egress: records each
    /// request and answers with one PNG, or panics when no call is allowed.
    struct FakeImageApi {
        image: Option<Vec<u8>>,
        seen: Mutex<Vec<EgressRequest>>,
    }

    #[async_trait::async_trait]
    impl EgressService for FakeImageApi {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            let image = self
                .image
                .clone()
                .expect("no image call may happen in this test");
            self.seen.lock().unwrap().push(request);
            Ok(EgressResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(&serde_json::json!({
                    "data": [{ "b64_json": STANDARD.encode(image), "revised_prompt": "a calm owl" }]
                }))
                .unwrap(),
            })
        }

        async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
            unreachable!("avatar generation never streams")
        }
    }

    fn encryption() -> Arc<EncryptionService> {
        Arc::new(
            EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
                .unwrap(),
        )
    }

    /// An owner context with provider services and the fake image API, and
    /// optionally an OpenAI account to draw with.
    async fn image_ctx(openai_account: bool, image: Option<Vec<u8>>) -> (Ctx, Arc<FakeImageApi>) {
        let mut ctx = owner_ctx().await;
        let encryption = encryption();
        if openai_account {
            ctx.db
                .create_provider(
                    DEFAULT_ORG_ID,
                    CreateProviderRow {
                        name: "openai".to_string(),
                        provider_type: "openai".to_string(),
                        base_url: Some("https://images.example/v1".to_string()),
                        api_key_encrypted: Some(encryption.encrypt_string("sk-test").unwrap()),
                        settings: None,
                    },
                )
                .await
                .unwrap();
        }
        let resolver = Arc::new(ProviderResolverService::new(
            ctx.db.clone(),
            Some(encryption.clone()),
        ));
        ctx.provider_service = Some(Arc::new(ProviderService::with_resolver(
            ctx.db.clone(),
            Some(encryption),
            resolver,
        )));
        let api = Arc::new(FakeImageApi {
            image,
            seen: Mutex::new(Vec::new()),
        });
        ctx.egress_service = Some(api.clone());
        (ctx, api)
    }

    fn generate(agent_id: &str, prompt: Option<&str>) -> GenerateAgentAvatar {
        GenerateAgentAvatar {
            agent_id: agent_id.to_string(),
            req: GenerateAgentAvatarRequest {
                prompt: prompt.map(str::to_string),
                quality: None,
            },
        }
    }

    #[tokio::test]
    async fn draws_with_the_org_account_and_sets_the_avatar() {
        let (ctx, api) = image_ctx(true, Some(png(512))).await;
        let agent_id = create_agent(&ctx, "owl-agent").await;

        let generated = generate(&agent_id, Some("A calm owl librarian"))
            .execute(&ctx)
            .await
            .expect("generate");

        assert_eq!(generated.model, "gpt-image-2.6-flare");
        assert_eq!(generated.provider_type, "openai");
        assert_eq!(generated.revised_prompt.as_deref(), Some("a calm owl"));
        assert!(
            generated.avatar.url.contains("square-256.png"),
            "{}",
            generated.avatar.url
        );
        assert_eq!(
            stored_source(&ctx, &agent_id).await.as_deref(),
            Some("generated")
        );

        let seen = api.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].url, "https://images.example/v1/images/generations");
        let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
        let prompt = body["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("A calm owl librarian"), "{prompt}");
        assert!(prompt.contains("square avatar icon"), "{prompt}");
        assert_eq!(body["size"], "1024x1024");
        assert_eq!(body["output_format"], "png");
    }

    #[tokio::test]
    async fn without_a_prompt_draws_from_the_agent_name() {
        let (ctx, api) = image_ctx(true, Some(png(256))).await;
        let agent_id = create_agent(&ctx, "triage-bot").await;

        generate(&agent_id, None)
            .execute(&ctx)
            .await
            .expect("generate");

        let seen = api.seen.lock().unwrap();
        let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
        let prompt = body["prompt"].as_str().unwrap();
        assert!(prompt.contains("AI agent named \"triage-bot\""), "{prompt}");
    }

    #[tokio::test]
    async fn without_an_image_account_fails_before_any_call() {
        let (ctx, _api) = image_ctx(false, None).await;
        let agent_id = create_agent(&ctx, "no-images").await;

        let err = generate(&agent_id, Some("anything"))
            .execute(&ctx)
            .await
            .unwrap_err();
        assert_bad_request(&err, "Image credentials are not configured");
        assert_eq!(stored_source(&ctx, &agent_id).await, None);
    }

    #[tokio::test]
    async fn rejects_bad_prompts_and_quality_before_any_call() {
        let (ctx, _api) = image_ctx(true, None).await;
        let agent_id = create_agent(&ctx, "picky").await;

        let err = generate(&agent_id, Some("   "))
            .execute(&ctx)
            .await
            .unwrap_err();
        assert_bad_request(&err, "prompt is empty");

        let long = "x".repeat(MAX_AVATAR_PROMPT_CHARS + 1);
        let err = generate(&agent_id, Some(&long))
            .execute(&ctx)
            .await
            .unwrap_err();
        assert_bad_request(&err, "at most 2000 characters");

        let err = GenerateAgentAvatar {
            agent_id: agent_id.clone(),
            req: GenerateAgentAvatarRequest {
                prompt: Some("an owl".to_string()),
                quality: Some("ultra".to_string()),
            },
        }
        .execute(&ctx)
        .await
        .unwrap_err();
        assert_bad_request(&err, "quality");
        assert_eq!(stored_source(&ctx, &agent_id).await, None);
    }

    #[tokio::test]
    async fn a_provider_failure_is_unavailable_and_keeps_the_old_avatar() {
        // A PNG too small to be an avatar stands in for a broken response:
        // rendering refuses it, so nothing is stored.
        let (ctx, _api) = image_ctx(true, Some(png(8))).await;
        let agent_id = create_agent(&ctx, "tiny").await;

        let err = generate(&agent_id, Some("an owl"))
            .execute(&ctx)
            .await
            .unwrap_err();
        assert_eq!(err.status().as_u16(), 400, "{}", err.message());
        assert_eq!(stored_source(&ctx, &agent_id).await, None);
    }

    #[tokio::test]
    async fn without_provider_services_is_unavailable() {
        let ctx = owner_ctx().await;
        let agent_id = create_agent(&ctx, "bare").await;
        let err = generate(&agent_id, Some("an owl"))
            .execute(&ctx)
            .await
            .unwrap_err();
        assert_eq!(err.status().as_u16(), 503, "{}", err.message());
    }

    #[tokio::test]
    async fn built_in_agent_rejects_generate_agent_avatar() {
        let (ctx, _api) = image_ctx(true, None).await;
        let agent_id = create_agent(&ctx, "chat").await;
        ctx.db
            .mark_agent_built_in(DEFAULT_ORG_ID, agent_id.parse().unwrap())
            .await
            .unwrap();

        let err = generate(&agent_id, Some("an owl"))
            .execute(&ctx)
            .await
            .unwrap_err();
        assert_bad_request(&err, "built-in agent");
    }

    #[tokio::test]
    async fn generate_agent_avatar_records_history_through_run() {
        let (ctx, _api) = image_ctx(true, Some(png(256))).await;
        let agent_id = create_agent(&ctx, "history-gen").await;

        generate(&agent_id, Some("an owl"))
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
            .find(|c| c.command == "generate_agent_avatar")
            .expect("history entry");
        assert_eq!(entry.action, "updated");
    }
}
