//! Image model profiles (`ServiceKind::Images`).
//!
//! Sources: OpenAI's image generation guide, model pages and pricing page
//! (developers.openai.com, read 2026-11-05), probed live against
//! `/v1/images/generations` the same day, and OpenRouter's
//! `GET /api/v1/images/models` for the gateway surface. Pricing follows the
//! text-oriented [`ModelCost`]: `input` is text input, `cache_read` cached text
//! input and `output` image output. The cost model has no image-input bucket,
//! so that rate is stated in the description. Models with no published price
//! keep `cost: None` rather than a guess.

use super::*;
use crate::model_profile_data::types::{GifLimits, ImageModelProfile};

/// GIF limits OpenAI documents for GPT Image 2.6 Flare and Sunburst:
/// 2-64 frames (default 9) at 20-2000 ms per frame (default 200).
pub(crate) const GPT_IMAGE_GIF: GifLimits = GifLimits {
    min_frames: 2,
    max_frames: 64,
    default_frames: 9,
    min_frame_ms: 20,
    max_frame_ms: 2000,
    default_frame_ms: 200,
};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

const GPT_SIZES: &[&str] = &["1024x1024", "1536x1024", "1024x1536", "auto"];
const RASTER: &[&str] = &["png", "jpeg", "webp"];

fn image_profile(
    name: &str,
    family: &str,
    description: &str,
    release_date: Option<&str>,
    cost: Option<ModelCost>,
    images: ImageModelProfile,
) -> ModelProfile {
    let mut profile = ModelProfile::minimal(name);
    profile.family = family.into();
    profile.description = Some(description.into());
    profile.release_date = release_date.map(Into::into);
    profile.last_updated = release_date.map(Into::into);
    profile.attachment = images.edits;
    profile.cost = cost;
    profile.modalities = Some(ModelModalities {
        input: if images.edits {
            vec![Modality::Text, Modality::Image]
        } else {
            vec![Modality::Text]
        },
        output: vec![Modality::Image],
    });
    profile.images = Some(images);
    profile
}

fn openai_image_cost(output: f64) -> ModelCost {
    let mut cost = ModelCost::new(5.0, output);
    cost.cache_read = Some(1.25);
    cost
}

/// GPT Image 2.5/2.6 capabilities. Both generations take `complexity`,
/// `markup` edits and the `xhigh`/`max` qualities; only 2.6 makes GIF and SVG.
fn gpt_image_2x(gif_and_svg: bool) -> ImageModelProfile {
    let mut formats = strings(RASTER);
    if gif_and_svg {
        formats.extend(strings(&["gif", "svg"]));
    }
    ImageModelProfile {
        formats,
        qualities: strings(&["low", "medium", "high", "xhigh", "max", "auto"]),
        sizes: strings(GPT_SIZES),
        edits: true,
        markup: true,
        complexity: true,
        max_count: 10,
        gif: gif_and_svg.then_some(GPT_IMAGE_GIF),
    }
}

fn gpt_image_classic() -> ImageModelProfile {
    ImageModelProfile {
        formats: strings(RASTER),
        qualities: strings(&["low", "medium", "high", "auto"]),
        sizes: strings(GPT_SIZES),
        edits: true,
        markup: false,
        complexity: false,
        max_count: 10,
        gif: None,
    }
}

fn single_image(edits: bool) -> ImageModelProfile {
    ImageModelProfile {
        formats: strings(&["png"]),
        qualities: Vec::new(),
        sizes: strings(&["1024x1024", "1536x1024", "1024x1536", "auto"]),
        edits,
        markup: false,
        complexity: false,
        max_count: 1,
        gif: None,
    }
}

pub(super) fn image_profile_data(model_id: &str) -> Option<ModelProfile> {
    Some(match model_id {
        "gpt-image-2.6-flare" => image_profile(
            "GPT Image 2.6 Flare",
            "gpt-image-2.6",
            "OpenAI's fast image model: images in about six seconds, animated GIFs and SVGs. Image input $8 per 1M tokens.",
            Some("2026-11-04"),
            Some(openai_image_cost(30.0)),
            gpt_image_2x(true),
        ),
        "gpt-image-2.6-sunburst" => image_profile(
            "GPT Image 2.6 Sunburst",
            "gpt-image-2.6",
            "OpenAI's detailed image model for dense compositions, text-heavy layouts and consistent edits, with animated GIFs and SVGs. Image input $8 per 1M tokens.",
            Some("2026-11-04"),
            Some(openai_image_cost(30.0)),
            gpt_image_2x(true),
        ),
        "gpt-image-2.5-flare" => image_profile(
            "GPT Image 2.5 Flare",
            "gpt-image-2.5",
            "OpenAI's fast GPT Image 2.5 model. Image input $8 per 1M tokens.",
            Some("2026-09-08"),
            Some(openai_image_cost(30.0)),
            gpt_image_2x(false),
        ),
        "gpt-image-2.5-sunburst" => image_profile(
            "GPT Image 2.5 Sunburst",
            "gpt-image-2.5",
            "OpenAI's detailed GPT Image 2.5 model. Image input $8 per 1M tokens.",
            Some("2026-09-08"),
            Some(openai_image_cost(30.0)),
            gpt_image_2x(false),
        ),
        "gpt-image-2" => image_profile(
            "GPT Image 2",
            "gpt-image-2",
            "OpenAI's ChatGPT Images 2.0 model. Image input $8 per 1M tokens.",
            Some("2026-04-21"),
            Some(openai_image_cost(30.0)),
            gpt_image_classic(),
        ),
        "gpt-image-1.5" => image_profile(
            "GPT Image 1.5",
            "gpt-image-1",
            "OpenAI's GPT Image 1.5 model. Image input $8 per 1M tokens.",
            None,
            Some(openai_image_cost(32.0)),
            gpt_image_classic(),
        ),
        "gpt-image-1-mini" => image_profile(
            "GPT Image 1 Mini",
            "gpt-image-1",
            "OpenAI's low-cost GPT Image 1 model. Image input $2.50 per 1M tokens.",
            None,
            Some({
                let mut cost = ModelCost::new(2.0, 8.0);
                cost.cache_read = Some(0.20);
                cost
            }),
            gpt_image_classic(),
        ),
        "gpt-image-1" => image_profile(
            "GPT Image 1",
            "gpt-image-1",
            "OpenAI's first GPT Image model.",
            None,
            None,
            gpt_image_classic(),
        ),
        "muse-image-1.0" => image_profile(
            "Muse Image",
            "muse-image",
            "Meta's Muse image model.",
            None,
            None,
            single_image(true),
        ),
        "gemini-3.1-flash-image" => image_profile(
            "Gemini 3.1 Flash Image",
            "gemini-3.1-flash-image",
            "Google's Nano Banana 2 image model, through OpenRouter's image API.",
            None,
            None,
            single_image(false),
        ),
        "gemini-3-pro-image" => image_profile(
            "Gemini 3 Pro Image",
            "gemini-3-pro-image",
            "Google's Nano Banana Pro image model, through OpenRouter's image API.",
            None,
            None,
            single_image(false),
        ),
        _ => return None,
    })
}

/// Mask image features a provider surface does not serve. OpenRouter's image
/// API takes only `png`, `jpeg` and `webp` and has no `complexity` or
/// `markup` fields (its `/api/v1/images/models` listing, 2026-11-05), so GPT
/// Image 2.6 loses GIF and SVG there. Only OpenAI and Azure OpenAI keep them.
pub(super) fn mask_for_surface(provider_type: &str, profile: &mut ModelProfile) {
    let Some(images) = profile.images.as_mut() else {
        return;
    };
    if provider_type == "openai" || provider_type == "azure_openai" {
        return;
    }
    images
        .formats
        .retain(|format| format != "gif" && format != "svg");
    images.gif = None;
    images.markup = false;
    images.complexity = false;
}
