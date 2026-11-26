//! Authored portrait bytes, never a deployment avatar ID or URL.
use super::{Result, error};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A portrait is separate from files seeded into the agent's workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum Avatar {
    /// Package-relative PNG, JPEG, GIF or WebP. No URLs or host paths.
    Source(String),
    Image(AvatarImage),
    /// Only false is valid: explicitly remove a destination avatar.
    Clear(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AvatarImage {
    pub content_type: String,
    pub image_base64: String,
}

pub const MAX_AVATAR_BYTES: usize = 5 * 1024 * 1024;

pub(super) fn image_type(path: &str) -> Option<&'static str> {
    match Path::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

pub(super) fn asset_limit(path: &str) -> usize {
    if image_type(path).is_some() {
        MAX_AVATAR_BYTES
    } else {
        super::MAX_FILE_BYTES
    }
}

impl Avatar {
    pub fn source(&self) -> Option<&str> {
        if let Self::Source(path) = self {
            Some(path)
        } else {
            None
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Source(path) => {
                if !super::assets::safe_source(path)
                    || !super::assets::allowed(Path::new(path))
                    || super::assets::asset_path(path).is_err()
                    || image_type(path).is_none()
                    || path.contains(['*', '?', '[', '{'])
                {
                    return Err(error(
                        "avatar",
                        "expected a safe relative image path, not a URL or credential path",
                    ));
                }
                Ok(())
            }
            Self::Image(image) => image.bytes().map(|_| ()),
            Self::Clear(false) => Ok(()),
            Self::Clear(true) => Err(error("avatar", "use false to remove an avatar")),
        }
    }
}

impl AvatarImage {
    pub fn new(content_type: impl Into<String>, bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
            return Err(error(
                "avatar",
                "expected a nonempty image of at most 5 MiB",
            ));
        }
        let image = Self {
            content_type: content_type.into(),
            image_base64: STANDARD.encode(bytes),
        };
        image.bytes()?;
        Ok(image)
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        if !matches!(
            self.content_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) {
            return Err(error(
                "avatar.content_type",
                "expected PNG, JPEG, GIF or WebP",
            ));
        }
        if self.image_base64.len() > MAX_AVATAR_BYTES.div_ceil(3) * 4 {
            return Err(error("avatar", "image exceeds 5 MiB"));
        }
        let bytes = STANDARD
            .decode(&self.image_base64)
            .map_err(|_| error("avatar.image_base64", "invalid base64"))?;
        if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
            return Err(error(
                "avatar",
                "expected a nonempty image of at most 5 MiB",
            ));
        }
        Ok(bytes)
    }
    pub(super) fn extension(&self) -> &'static str {
        match self.content_type.as_str() {
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "png",
        }
    }
}

// Git trees and native folders cannot contain both a file and descendants.
// A new portrait must not replace an unrelated directory with a blob.
pub(super) fn path_taken(
    entries: &std::collections::BTreeMap<String, Vec<u8>>,
    path: &str,
) -> bool {
    let prefix = format!("{path}/");
    entries.keys().any(|existing| {
        existing == path
            || existing.starts_with(&prefix)
            || path.starts_with(&format!("{existing}/"))
    })
}
