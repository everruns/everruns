// Agent avatar rendering: one upload in, every served variant out.
//
// Design decisions:
// - All variants are rendered once, at upload time, and stored. Serving is a
//   single keyed read with immutable cache headers; nothing is resized on the
//   request path. A replacement gets a fresh avatar id, so a URL never changes
//   meaning and caches (browsers, CDNs, Slack's image proxy) can keep it forever.
// - The upload is center-cropped to a square once. "Square" is the canonical
//   avatar; "circle" is the same square with an anti-aliased circular alpha
//   mask, for surfaces that cannot clip themselves (Slack, email, A2A clients).
// - Everything is PNG. Slack and most A2A/MCP clients do not accept WebP, and
//   the circle needs transparency, which rules out JPEG.
// - `source.png` keeps the square crop at up to 1024px so later presets (or a
//   new size) can be re-rendered without asking the user to upload again.
//   Images smaller than the largest preset are upscaled for that preset rather
//   than refused, because Slack requires a 512px icon.

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage};
use std::io::Cursor;

/// Preset edge lengths, in pixels, rendered for both shapes.
pub const AVATAR_SIZES: [u32; 5] = [32, 64, 128, 256, 512];
/// Largest accepted upload.
pub const MAX_AVATAR_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Smallest accepted edge after the square crop.
pub const MIN_AVATAR_EDGE: u32 = 64;
/// Edge of the stored square source.
const SOURCE_MAX_EDGE: u32 = 1024;
/// Decoder bound, same as message images: rejects decompression bombs.
const MAX_AVATAR_PIXELS: u64 = 40_000_000;

pub const AVATAR_CONTENT_TYPE: &str = "image/png";
pub const SOURCE_VARIANT: &str = "source.png";
/// Size used where one URL has to stand for the avatar (Agent Card, API `url`).
pub const DEFAULT_AVATAR_SIZE: u32 = 256;
/// Size Slack requires for an app icon (512 to 2000 px).
pub const SLACK_ICON_SIZE: u32 = 512;

/// Avatar shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarShape {
    Square,
    Circle,
}

impl AvatarShape {
    fn as_str(self) -> &'static str {
        match self {
            AvatarShape::Square => "square",
            AvatarShape::Circle => "circle",
        }
    }
}

/// Stored variant name for a preset, e.g. `square-128.png`.
pub fn variant_name(shape: AvatarShape, size: u32) -> String {
    format!("{}-{size}.png", shape.as_str())
}

/// Whether `name` is a variant this server renders. Anything else is a 404
/// without touching storage.
pub fn is_known_variant(name: &str) -> bool {
    if name == SOURCE_VARIANT {
        return true;
    }
    let Some(stem) = name.strip_suffix(".png") else {
        return false;
    };
    let Some((shape, size)) = stem.split_once('-') else {
        return false;
    };
    matches!(shape, "square" | "circle")
        && size
            .parse::<u32>()
            .is_ok_and(|parsed| AVATAR_SIZES.contains(&parsed) && parsed.to_string() == size)
}

/// One rendered, encoded variant.
#[derive(Debug, Clone)]
pub struct RenderedAvatarVariant {
    pub variant: String,
    pub content_type: &'static str,
    pub data: Vec<u8>,
}

/// Why an upload was refused. Messages are safe to show the user.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AvatarError {
    #[error("Avatar must be a PNG, JPEG, GIF or WebP image")]
    UnsupportedType,
    #[error("Avatar must be at most 10 MB")]
    TooLarge,
    #[error("Avatar could not be read as an image")]
    Undecodable,
    #[error("Avatar must be at least 64x64 pixels")]
    TooSmall,
}

fn format_for(content_type: &str) -> Option<ImageFormat> {
    let bare = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match bare.as_str() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

/// Decode an upload and render the source plus every preset in both shapes.
pub fn render_avatar(
    data: &[u8],
    content_type: &str,
) -> Result<Vec<RenderedAvatarVariant>, AvatarError> {
    if data.len() > MAX_AVATAR_UPLOAD_BYTES {
        return Err(AvatarError::TooLarge);
    }
    let declared = format_for(content_type).ok_or(AvatarError::UnsupportedType)?;
    // The bytes decide, not the header: a mislabeled file is refused rather
    // than handed to a decoder for another format.
    if image::guess_format(data).ok() != Some(declared) {
        return Err(AvatarError::Undecodable);
    }
    let max_dim = (MAX_AVATAR_PIXELS as f64).sqrt() as u32;
    let mut reader = ImageReader::with_format(Cursor::new(data), declared);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_dim);
    limits.max_image_height = Some(max_dim);
    limits.max_alloc = Some(MAX_AVATAR_PIXELS * 4);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| AvatarError::Undecodable)?;

    let square = center_square(&decoded);
    if square.width() < MIN_AVATAR_EDGE {
        return Err(AvatarError::TooSmall);
    }
    let source = if square.width() > SOURCE_MAX_EDGE {
        image::imageops::resize(
            &square,
            SOURCE_MAX_EDGE,
            SOURCE_MAX_EDGE,
            FilterType::Lanczos3,
        )
    } else {
        square
    };

    let mut out = Vec::with_capacity(1 + AVATAR_SIZES.len() * 2);
    out.push(encode(SOURCE_VARIANT.to_string(), &source)?);
    for size in AVATAR_SIZES {
        let resized = if source.width() == size {
            source.clone()
        } else {
            image::imageops::resize(&source, size, size, FilterType::Lanczos3)
        };
        let circle = circle_mask(&resized);
        out.push(encode(variant_name(AvatarShape::Square, size), &resized)?);
        out.push(encode(variant_name(AvatarShape::Circle, size), &circle)?);
    }
    Ok(out)
}

fn center_square(image: &DynamicImage) -> RgbaImage {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let edge = width.min(height);
    let x = (width - edge) / 2;
    let y = (height - edge) / 2;
    image::imageops::crop_imm(&rgba, x, y, edge, edge).to_image()
}

/// Multiply alpha by pixel coverage of the inscribed circle, so the edge is
/// anti-aliased instead of stair-stepped at small sizes.
fn circle_mask(square: &RgbaImage) -> RgbaImage {
    let edge = square.width() as f32;
    let radius = edge / 2.0;
    let mut out = square.clone();
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - radius;
        let dy = y as f32 + 0.5 - radius;
        let distance = (dx * dx + dy * dy).sqrt();
        let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
        let Rgba([r, g, b, a]) = *pixel;
        *pixel = Rgba([r, g, b, (a as f32 * coverage).round() as u8]);
    }
    out
}

fn encode(variant: String, image: &RgbaImage) -> Result<RenderedAvatarVariant, AvatarError> {
    let mut buffer = Cursor::new(Vec::new());
    image
        .write_to(&mut buffer, ImageFormat::Png)
        .map_err(|_| AvatarError::Undecodable)?;
    Ok(RenderedAvatarVariant {
        variant,
        content_type: AVATAR_CONTENT_TYPE,
        data: buffer.into_inner(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let mut buffer = Cursor::new(Vec::new());
        image.write_to(&mut buffer, ImageFormat::Png).unwrap();
        buffer.into_inner()
    }

    fn decode(data: &[u8]) -> RgbaImage {
        image::load_from_memory(data).unwrap().to_rgba8()
    }

    #[test]
    fn renders_source_and_every_preset_in_both_shapes() {
        let variants = render_avatar(&png(300, 200), "image/png").unwrap();
        let names: Vec<&str> = variants.iter().map(|v| v.variant.as_str()).collect();
        assert_eq!(names.len(), 1 + AVATAR_SIZES.len() * 2);
        assert_eq!(names[0], SOURCE_VARIANT);
        for size in AVATAR_SIZES {
            for shape in [AvatarShape::Square, AvatarShape::Circle] {
                let name = variant_name(shape, size);
                let variant = variants.iter().find(|v| v.variant == name).unwrap();
                assert_eq!(variant.content_type, "image/png");
                assert_eq!(decode(&variant.data).dimensions(), (size, size), "{name}");
            }
        }
    }

    #[test]
    fn crops_the_center_square() {
        let variants = render_avatar(&png(300, 200), "image/png").unwrap();
        let source = decode(&variants[0].data);
        assert_eq!(source.dimensions(), (200, 200));
        // A 300px-wide image split red|blue at x=150 is cropped to x in
        // 50..250, so the split stays in the middle of the square.
        assert_eq!(source.get_pixel(10, 100).0, [255, 0, 0, 255]);
        assert_eq!(source.get_pixel(190, 100).0, [0, 0, 255, 255]);
    }

    #[test]
    fn circle_is_transparent_in_the_corners_and_opaque_in_the_middle() {
        let variants = render_avatar(&png(128, 128), "image/png").unwrap();
        let circle = variants
            .iter()
            .find(|v| v.variant == "circle-128.png")
            .unwrap();
        let image = decode(&circle.data);
        assert_eq!(image.get_pixel(0, 0).0[3], 0);
        assert_eq!(image.get_pixel(127, 127).0[3], 0);
        assert_eq!(image.get_pixel(64, 64).0[3], 255);
        let square = variants
            .iter()
            .find(|v| v.variant == "square-128.png")
            .unwrap();
        assert_eq!(decode(&square.data).get_pixel(0, 0).0[3], 255);
    }

    #[test]
    fn large_uploads_are_bounded_and_small_ones_upscaled() {
        let large = render_avatar(&png(1500, 1500), "image/png").unwrap();
        assert_eq!(decode(&large[0].data).dimensions(), (1024, 1024));
        let small = render_avatar(&png(64, 80), "image/png").unwrap();
        let icon = small
            .iter()
            .find(|v| v.variant == "square-512.png")
            .unwrap();
        assert_eq!(decode(&icon.data).dimensions(), (512, 512));
    }

    #[test]
    fn refuses_bad_uploads() {
        assert_eq!(
            render_avatar(&png(100, 100), "image/svg+xml").unwrap_err(),
            AvatarError::UnsupportedType
        );
        assert_eq!(
            render_avatar(&png(100, 100), "image/jpeg").unwrap_err(),
            AvatarError::Undecodable
        );
        assert_eq!(
            render_avatar(b"not an image", "image/png").unwrap_err(),
            AvatarError::Undecodable
        );
        assert_eq!(
            render_avatar(&png(200, 40), "image/png").unwrap_err(),
            AvatarError::TooSmall
        );
        let oversized = vec![0u8; MAX_AVATAR_UPLOAD_BYTES + 1];
        assert_eq!(
            render_avatar(&oversized, "image/png").unwrap_err(),
            AvatarError::TooLarge
        );
    }

    #[test]
    fn only_rendered_variants_are_known() {
        assert!(is_known_variant("source.png"));
        assert!(is_known_variant("square-32.png"));
        assert!(is_known_variant("circle-512.png"));
        assert!(!is_known_variant("square-100.png"));
        assert!(!is_known_variant("square-064.png"));
        assert!(!is_known_variant("oval-64.png"));
        assert!(!is_known_variant("square-64.jpg"));
        assert!(!is_known_variant("../square-64.png"));
    }
}
