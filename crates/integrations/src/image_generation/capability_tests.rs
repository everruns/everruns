use super::capability::{ImageGenerationConfig, parse_capability_config};
use super::outputs::{gif_preview, normalize_workspace_path, output_filename};
use super::resolve::{ResolvedImageModel, api_model_id, resolve_image_model};
use super::tools::{Animation, CommonImageArgs, EditImageArgs, GenerateImageTool, build_request};
use super::*;
use async_trait::async_trait;
use base64::Engine;
use everruns_contracts::error::Result;
use everruns_contracts::model_profile_data::get_model_profile;
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::connection_services::{
    ImageModelBinding, ProviderCredentialStore, ProviderCredentials,
};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::runtime::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use everruns_contracts::typed_id::{ImageId, SessionId};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct MockStore {
    providers: Vec<(&'static str, ProviderCredentials)>,
    /// Org default image model; a catalog id resolves to it as well.
    image_model: Option<ImageModelBinding>,
}

#[async_trait]
impl ProviderCredentialStore for MockStore {
    async fn get_image_model(
        &self,
        model_id: Option<&str>,
        _session_id: SessionId,
    ) -> Result<Option<ImageModelBinding>> {
        Ok(self
            .image_model
            .clone()
            .filter(|binding| model_id.is_none_or(|id| id == binding.model_id)))
    }

    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        Ok(self
            .providers
            .iter()
            .find(|(name, _)| *name == provider_type)
            .map(|(_, credentials)| credentials.clone()))
    }
}

fn credentials(key: &str) -> ProviderCredentials {
    ProviderCredentials {
        api_key: key.to_string(),
        base_url: None,
    }
}

fn context(providers: Vec<(&'static str, ProviderCredentials)>) -> ToolContext {
    context_with(providers, None)
}

fn context_with(
    providers: Vec<(&'static str, ProviderCredentials)>,
    image_model: Option<ImageModelBinding>,
) -> ToolContext {
    let session_id = SessionId::new();
    ToolContext {
        provider_credential_store: Some(Arc::new(MockStore {
            providers,
            image_model,
        })),
        ..ToolContext::new(session_id)
    }
}

fn binding(provider_type: &str, model: &str) -> ImageModelBinding {
    ImageModelBinding {
        model_id: "model_0193a0000000700080000000000000aa".to_string(),
        provider_id: "prov_1".to_string(),
        provider_type: provider_type.to_string(),
        model: model.to_string(),
        profile_key: format!("openai/{model}"),
        api_key: "org-key".to_string(),
        base_url: None,
        headers: [("x-org".to_string(), "1".to_string())].into(),
    }
}

fn config(value: Value) -> ImageGenerationConfig {
    parse_capability_config(&value).unwrap()
}

async fn resolve(
    context: &ToolContext,
    settings: Value,
    format: &str,
) -> std::result::Result<ResolvedImageModel, String> {
    resolve_image_model(context, None, &config(settings), format)
        .await
        .map_err(|error| match error {
            ToolExecutionResult::ToolError(message) => message,
            other => format!("{other:?}"),
        })
}

#[tokio::test]
async fn platform_default_is_flare_on_openai() {
    let ctx = context(vec![
        ("openrouter", credentials("or")),
        ("openai", credentials("oa")),
    ]);
    let resolved = resolve(&ctx, json!({}), "png").await.unwrap();
    assert_eq!(resolved.provider_type, "openai");
    assert_eq!(resolved.api_model, PLATFORM_DEFAULT_IMAGE_MODEL);
    assert!(resolved.profile.unwrap().gif.is_some());
}

#[tokio::test]
async fn azure_serves_gpt_image_when_openai_is_missing() {
    let ctx = context(vec![(
        "azure_openai",
        ProviderCredentials {
            api_key: "az".to_string(),
            base_url: Some("https://res.openai.azure.com/openai/v1".to_string()),
        },
    )]);
    let resolved = resolve(&ctx, json!({ "model": "gpt-image-2" }), "png")
        .await
        .unwrap();
    assert_eq!(resolved.provider_type, "azure_openai");
    assert_eq!(resolved.api_key, "az");
}

#[tokio::test]
async fn openrouter_serves_gpt_image_under_its_vendor_prefix() {
    let ctx = context(vec![("openrouter", credentials("or"))]);
    let resolved = resolve(&ctx, json!({}), "png").await.unwrap();
    assert_eq!(resolved.provider_type, "openrouter");
    assert_eq!(resolved.api_model, "openai/gpt-image-2.6-flare");
    // OpenRouter does not serve GIF, so its masked profile drops it.
    assert!(resolved.profile.unwrap().gif.is_none());
}

#[tokio::test]
async fn gif_skips_providers_that_cannot_animate() {
    let ctx = context(vec![("openrouter", credentials("or"))]);
    let error = resolve(&ctx, json!({}), "gif").await.unwrap_err();
    assert!(error.contains("gif output needs GPT Image 2.6"), "{error}");

    let ctx = context(vec![
        ("openrouter", credentials("or")),
        ("openai", credentials("oa")),
    ]);
    assert_eq!(
        resolve(&ctx, json!({}), "gif").await.unwrap().provider_type,
        "openai"
    );
}

#[tokio::test]
async fn gpt_image_falls_back_to_muse_on_meta() {
    let ctx = context(vec![("meta", credentials("meta"))]);
    let resolved = resolve(&ctx, json!({}), "png").await.unwrap();
    assert_eq!(resolved.provider_type, "meta");
    assert_eq!(resolved.api_model, "muse-image-1.0");

    let error = resolve(&ctx, json!({ "fallback": "off" }), "png")
        .await
        .unwrap_err();
    assert!(error.contains("Image credentials are not configured"));
}

#[tokio::test]
async fn muse_ignores_openai_and_uses_openrouter_alias() {
    let ctx = context(vec![("openai", credentials("oa"))]);
    assert!(
        resolve(&ctx, json!({ "model": "muse-image-1.0" }), "png")
            .await
            .is_err()
    );
    let ctx = context(vec![("openrouter", credentials("or"))]);
    let resolved = resolve(&ctx, json!({ "model": "muse-image-1.0" }), "png")
        .await
        .unwrap();
    assert_eq!(resolved.api_model, "meta/muse-image");
}

#[tokio::test]
async fn org_default_wins_over_platform_default() {
    let ctx = context_with(
        vec![("openai", credentials("oa"))],
        Some(binding("openai", "gpt-image-2.6-sunburst")),
    );
    let resolved = resolve(&ctx, json!({}), "gif").await.unwrap();
    assert_eq!(resolved.api_model, "gpt-image-2.6-sunburst");
    assert_eq!(resolved.api_key, "org-key");
    assert_eq!(resolved.headers, vec![("x-org".into(), "1".into())]);
}

#[tokio::test]
async fn org_default_that_cannot_animate_explains_why() {
    let ctx = context_with(
        vec![("openai", credentials("oa"))],
        Some(binding("openai", "gpt-image-2")),
    );
    let error = resolve(&ctx, json!({}), "gif").await.unwrap_err();
    assert!(error.contains("cannot make gif"), "{error}");
}

#[tokio::test]
async fn catalog_model_never_falls_back_to_another_account() {
    let ctx = context(vec![("openai", credentials("oa"))]);
    let error = resolve(
        &ctx,
        json!({ "model": "model_0193a0000000700080000000000000bb" }),
        "png",
    )
    .await
    .unwrap_err();
    assert!(error.contains("not available in this organization"), "{error}");
}

#[test]
fn api_model_ids_follow_openrouter_namespaces() {
    assert_eq!(api_model_id("openai", "gpt-image-2"), "gpt-image-2");
    assert_eq!(
        api_model_id("openrouter", "gemini-3-pro-image"),
        "google/gemini-3-pro-image"
    );
    assert_eq!(
        api_model_id("openrouter", "muse-image-1.0"),
        "meta/muse-image"
    );
}

#[test]
fn config_accepts_legacy_settings_and_ignores_unknown_fields() {
    let parsed = config(json!({
        "model": "gpt-image-1",
        "default_quality": "high",
        "partial_images": 3,
        "future_field": true
    }));
    assert_eq!(parsed.model.as_deref(), Some("gpt-image-1"));
    assert_eq!(parsed.quality(), "high");
    assert_eq!(config(json!({ "model": "  " })).model, None);
}

#[test]
fn config_rejects_bad_values() {
    for (settings, needle) in [
        (json!({ "model": "chatgpt-image-latest" }), "unknown image model"),
        (json!({ "partial_images": 4 }), "partial_images"),
        (json!({ "gif_frames": 65 }), "gif_frames"),
        (json!({ "gif_frame_ms": 10 }), "gif_frame_ms"),
        (json!({ "default_format": "bmp" }), "default_format"),
        (json!({ "complexity": "extreme" }), "complexity"),
    ] {
        let error = ImageGenerationCapability::new()
            .validate_config(&settings)
            .unwrap_err();
        assert!(error.contains(needle), "{settings}: {error}");
    }
}

fn resolved(provider_type: &str, model: &str) -> ResolvedImageModel {
    ResolvedImageModel {
        provider_type: provider_type.to_string(),
        api_key: "k".to_string(),
        base_url: None,
        headers: Vec::new(),
        api_model: model.to_string(),
        profile: get_model_profile(provider_type, model).and_then(|profile| profile.images),
    }
}

fn args(value: Value) -> CommonImageArgs {
    serde_json::from_value(value).unwrap()
}

#[test]
fn gif_request_uses_profile_defaults_and_never_streams() {
    let request = build_request(
        "lantern",
        &args(json!({ "format": "gif", "size": "1024x1024" })),
        &ImageGenerationConfig::default(),
        &resolved("openai", "gpt-image-2.6-flare"),
        Animation::default(),
    )
    .unwrap();
    // The frame count sets a GIF's size; explicit sizes overflow the sheet.
    assert_eq!(request.size, None);
    assert_eq!(request.frame_count, Some(9));
    assert_eq!(request.frame_duration_ms, Some(200));
    assert_eq!(request.partial_images, None);
    assert_eq!(request.quality.as_deref(), Some("medium"));

    let request = build_request(
        "lantern",
        &args(json!({ "format": "gif" })),
        &config(json!({ "gif_frames": 12, "complexity": "low" })),
        &resolved("openai", "gpt-image-2.6-flare"),
        Animation {
            frames: None,
            frame_duration_ms: Some(80),
        },
    )
    .unwrap();
    assert_eq!(request.frame_count, Some(12));
    assert_eq!(request.frame_duration_ms, Some(80));
    assert_eq!(request.complexity.as_deref(), Some("low"));
}

#[test]
fn requests_are_checked_against_the_model() {
    let flare = resolved("openai", "gpt-image-2.6-flare");
    let classic = resolved("openai", "gpt-image-2");
    let defaults = ImageGenerationConfig::default();
    let reject = |common: Value, model: &ResolvedImageModel, animation: Animation| {
        build_request("p", &args(common), &defaults, model, animation).unwrap_err()
    };
    assert!(
        reject(
            json!({ "format": "gif" }),
            &flare,
            Animation {
                frames: Some(80),
                frame_duration_ms: None
            }
        )
        .contains("frames must be between 2 and 64")
    );
    assert!(
        reject(
            json!({}),
            &flare,
            Animation {
                frames: Some(4),
                frame_duration_ms: None
            }
        )
        .contains("only apply to format gif")
    );
    assert!(reject(json!({ "format": "svg", "count": 2 }), &flare, Animation::default()).contains("count to 1"));
    assert!(reject(json!({ "quality": "xhigh" }), &classic, Animation::default()).contains("xhigh"));
    assert!(reject(json!({ "complexity": "low" }), &classic, Animation::default()).contains("complexity"));

    // Configured options a model lacks fall back to its defaults.
    let request = build_request(
        "p",
        &args(json!({})),
        &config(json!({ "default_quality": "max", "complexity": "low" })),
        &classic,
        Animation::default(),
    )
    .unwrap();
    assert_eq!(request.quality, None);
    assert_eq!(request.complexity, None);
    assert_eq!(request.partial_images, Some(1));

    // Muse takes no quality at all and one image per call.
    let muse = resolved("meta", "muse-image-1.0");
    let request = build_request("p", &args(json!({})), &defaults, &muse, Animation::default())
        .unwrap();
    assert_eq!(request.quality, None);
    assert!(reject(json!({ "count": 2 }), &muse, Animation::default()).contains("between 1 and 1"));
}

#[test]
fn edit_args_parse_image_ids_and_markup() {
    let id = ImageId::new();
    let args: EditImageArgs = serde_json::from_value(json!({
        "prompt": "edit",
        "image_id": id.to_string(),
        "markup_path": "/workspace/notes.png"
    }))
    .unwrap();
    assert_eq!(args.image_id, Some(id));
    assert_eq!(args.markup_path.as_deref(), Some("/workspace/notes.png"));
}

#[test]
fn paths_and_filenames() {
    assert_eq!(normalize_workspace_path("/workspace/out"), "/out");
    assert_eq!(normalize_workspace_path("out/file.png"), "/out/file.png");
    assert_eq!(output_filename("image", 0, 1, "gif"), "image.gif");
    assert_eq!(output_filename("image", 1, 3, "jpeg"), "image-2.jpg");
}

/// A three-frame 4x4 GIF.
fn tiny_gif() -> Vec<u8> {
    use image::{Delay, Frame, Rgba, RgbaImage, codecs::gif::GifEncoder};
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut bytes);
        for shade in [0u8, 120, 240] {
            let buffer = RgbaImage::from_pixel(4, 4, Rgba([shade, 80, 160, 255]));
            encoder
                .encode_frame(Frame::from_parts(
                    buffer,
                    0,
                    0,
                    Delay::from_numer_denom_ms(200, 1),
                ))
                .unwrap();
        }
    }
    bytes
}

#[test]
fn gif_preview_is_the_first_frame_as_png() {
    let (png, frames) = gif_preview(&tiny_gif()).unwrap();
    assert_eq!(frames, 3);
    assert!(png.starts_with(b"\x89PNG"));
    assert!(gif_preview(b"not a gif").is_none());
}

/// Answers every image request with one canned JSON body.
struct CannedEgress {
    body: Vec<u8>,
    seen: Mutex<Vec<EgressRequest>>,
}

#[async_trait]
impl EgressService for CannedEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        self.seen.lock().unwrap().push(request);
        Ok(EgressResponse {
            status: 200,
            headers: Default::default(),
            body: self.body.clone(),
        })
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        self.seen.lock().unwrap().push(request);
        unreachable!("GIF requests are never streamed")
    }
}

#[tokio::test]
async fn generated_gif_reaches_the_model_as_a_png_frame() {
    let gif = base64::engine::general_purpose::STANDARD.encode(tiny_gif());
    let egress = Arc::new(CannedEgress {
        body: serde_json::to_vec(&json!({
            "data": [{ "b64_json": gif }],
            "usage": { "input_tokens": 12, "output_tokens": 900 }
        }))
        .unwrap(),
        seen: Mutex::new(Vec::new()),
    });
    let ctx = context(vec![("openai", credentials("sk"))]).with_egress_service(egress.clone());
    let result = GenerateImageTool::new(json!({}))
        .execute_with_context(
            json!({ "prompt": "lantern", "format": "gif", "frames": 3, "persist_artifact": false }),
            &ctx,
        )
        .await;
    let ToolExecutionResult::SuccessWithImages { result, images } = result else {
        panic!("expected images, got {result:?}");
    };
    assert_eq!(images[0].media_type, "image/png");
    assert_eq!(result["model"], "gpt-image-2.6-flare");
    assert_eq!(result["images"][0]["media_type"], "image/gif");
    assert_eq!(result["images"][0]["frames"], 3);
    assert_eq!(result["usage"]["output_tokens"], 900);

    let sent: Value = serde_json::from_slice(&egress.seen.lock().unwrap()[0].body).unwrap();
    assert_eq!(sent["output_format"], "gif");
    assert_eq!(sent["frame_count"], 3);
    assert!(sent.get("stream").is_none());
}

#[tokio::test]
async fn system_prompt_within_budget() {
    let ctx = everruns_contracts::runtime::capabilities::SystemPromptContext::without_file_store(
        SessionId::new(),
    );
    let prompt = ImageGenerationCapability::new()
        .system_prompt_contribution(&ctx)
        .await
        .unwrap();
    assert!(prompt.len() <= 850, "prompt is {} bytes", prompt.len());
}
