// Images domain — commands, queries, types.

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

/// Image MIME types the server stores and serves inline.
pub(crate) const ALLOWED_CONTENT_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Strip MIME parameters (`; charset=...`) and lowercase the bare media type
/// so callers tolerate common header variations.
pub(crate) fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}
