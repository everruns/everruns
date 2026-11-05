//! Live image generation through `OpenAIImagesDriver`: GPT Image 2.6 stills,
//! animated GIFs and markup edits on OpenAI, and Flare through OpenRouter's
//! image API. Small, low-quality requests keep each run cheap.

use async_trait::async_trait;
use base64::Engine;
use everruns_contracts::images::{
    ImageInput, ImageRequest, ImageResponse, ImagesDriver, ImagesDriverError, ImagesHttpBody,
    ImagesHttpRequest, ImagesHttpResponse, ImagesTransport,
};
use everruns_drivers::openai::images::OpenAIImagesDriver;
use futures::StreamExt;
use std::sync::Arc;

/// Plain reqwest transport. Hosts send through their egress service; this
/// test only needs the wire. Redirects are not followed, like the host's.
struct ReqwestTransport(reqwest::Client);

#[async_trait]
impl ImagesTransport for ReqwestTransport {
    async fn send(
        &self,
        request: ImagesHttpRequest,
    ) -> Result<ImagesHttpResponse, ImagesDriverError> {
        let mut builder = self
            .0
            .post(&request.url)
            .timeout(std::time::Duration::from_millis(request.timeout_ms))
            .body(request.body);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| ImagesDriverError(error.to_string()))?;
        let status = response.status().as_u16();
        let body = if request.stream {
            ImagesHttpBody::Stream(
                response
                    .bytes_stream()
                    .map(|chunk| chunk.map(|bytes| bytes.to_vec()).map_err(|e| e.to_string()))
                    .boxed(),
            )
        } else {
            ImagesHttpBody::Full(
                response
                    .bytes()
                    .await
                    .map_err(|error| ImagesDriverError(error.to_string()))?
                    .to_vec(),
            )
        };
        Ok(ImagesHttpResponse { status, body })
    }
}

fn driver(provider_type: &str, key_var: &str) -> OpenAIImagesDriver {
    let key = std::env::var(key_var)
        .unwrap_or_else(|_| panic!("{key_var} must be set for the live image tests"));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    OpenAIImagesDriver::new(Arc::new(ReqwestTransport(client)), provider_type, key, None)
}

fn decode(response: &ImageResponse) -> Vec<u8> {
    let image = response.images.first().expect("one image");
    base64::engine::general_purpose::STANDARD
        .decode(&image.b64_json)
        .expect("base64 image")
}

/// Retry transport drops only; a provider or contract error fails at once.
async fn create(driver: &OpenAIImagesDriver, request: ImageRequest) -> ImageResponse {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match driver.create_images(request.clone(), None).await {
            Ok(response) => return response,
            Err(error)
                if attempt < super::TRANSPORT_ATTEMPTS
                    && super::is_transient_transport(&error.0) =>
            {
                eprintln!("transient image transport error, retrying: {}", error.0);
            }
            Err(error) => panic!("image request failed: {}", error.0),
        }
    }
}

fn still(model: &str) -> ImageRequest {
    ImageRequest {
        model: model.to_string(),
        prompt: "A single red apple on a white table, flat illustration".to_string(),
        size: Some("1024x1024".to_string()),
        quality: Some("low".to_string()),
        output_format: Some("png".to_string()),
        count: 1,
        ..Default::default()
    }
}

#[tokio::test]
async fn flare_draws_a_png() {
    let mut request = still("gpt-image-2.6-flare");
    request.complexity = Some("low".to_string());
    let response = create(&driver("openai", "OPENAI_API_KEY"), request).await;
    assert!(decode(&response).starts_with(b"\x89PNG"));
    assert!(response.usage.is_some_and(|usage| usage.output_tokens > 0));
}

#[tokio::test]
async fn flare_animates_a_gif() {
    let request = ImageRequest {
        model: "gpt-image-2.6-flare".to_string(),
        prompt: "A small blue ball bouncing on a white floor, simple flat style".to_string(),
        quality: Some("low".to_string()),
        output_format: Some("gif".to_string()),
        complexity: Some("low".to_string()),
        frame_count: Some(4),
        frame_duration_ms: Some(150),
        count: 1,
        ..Default::default()
    };
    let response = create(&driver("openai", "OPENAI_API_KEY"), request).await;
    let gif = decode(&response);
    assert!(gif.starts_with(b"GIF89a"), "not a GIF");
    use image::AnimationDecoder;
    let frames = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&gif))
        .unwrap()
        .into_frames()
        .count();
    assert!(frames > 1, "a GIF of {frames} frame(s) is not animated");
}

#[tokio::test]
async fn sunburst_edits_with_markup() {
    let png = |pixel: image::Rgba<u8>| {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(256, 256, pixel))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    };
    let mut markup = image::RgbaImage::from_pixel(256, 256, image::Rgba([0, 0, 0, 0]));
    for x in 96..160 {
        for y in 96..160 {
            markup.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
        }
    }
    let mut markup_png = Vec::new();
    image::DynamicImage::ImageRgba8(markup)
        .write_to(
            &mut std::io::Cursor::new(&mut markup_png),
            image::ImageFormat::Png,
        )
        .unwrap();
    let request = ImageRequest {
        model: "gpt-image-2.6-sunburst".to_string(),
        prompt: "Put a yellow star where the red square is marked".to_string(),
        quality: Some("low".to_string()),
        output_format: Some("png".to_string()),
        count: 1,
        sources: vec![ImageInput {
            filename: "canvas.png".to_string(),
            media_type: "image/png".to_string(),
            data: png(image::Rgba([240, 240, 255, 255])),
        }],
        markup: Some(ImageInput {
            filename: "markup.png".to_string(),
            media_type: "image/png".to_string(),
            data: markup_png,
        }),
        ..Default::default()
    };
    let response = create(&driver("openai", "OPENAI_API_KEY"), request).await;
    assert!(decode(&response).starts_with(b"\x89PNG"));
}

#[tokio::test]
async fn openrouter_serves_flare() {
    let response = create(
        &driver("openrouter", "OPENROUTER_API_KEY"),
        still("openai/gpt-image-2.6-flare"),
    )
    .await;
    let bytes = decode(&response);
    assert!(
        bytes.starts_with(b"\x89PNG") || bytes.starts_with(b"\xff\xd8"),
        "not an image"
    );
}
