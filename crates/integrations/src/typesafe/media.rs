//! Session files attached to a `jev_decision` call.
//!
//! Decision: the tool takes session file paths, not URLs. A file is read from
//! the session's own file store and sent inline, so the vendor never fetches
//! anything on the agent's behalf (an agent-supplied URL would bypass the
//! session's network policy). Only drivers that declare media support
//! (Gemini Decision 1) accept the result; the others reject it before any
//! round trip.

use everruns_contracts::decisions::{DecisionMedia, DecisionRequest};
use everruns_contracts::runtime::SessionFile;
use everruns_contracts::runtime::tool_context::ToolContext;

/// Files per call.
pub(crate) const MAX_FILES: usize = 8;
/// Raw bytes across all files: 15 MiB, about 20 MiB once base64-encoded, the
/// Gemini driver's media cap.
const MAX_TOTAL_BYTES: usize = 15 * 1024 * 1024;

/// The media type for a path, by extension; `None` when the decision models
/// cannot read it.
pub(crate) fn media_type(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mpeg" | "mpg" => "video/mpeg",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

/// `/workspace/a.png`, `a.png` and `/a.png` all name the file store's `/a.png`.
fn store_path(path: &str) -> String {
    let path = path.trim();
    let path = path.strip_prefix("/workspace").unwrap_or(path);
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Read `files` from the session and attach them to `request`, in order.
pub(crate) async fn attach_files(
    mut request: DecisionRequest,
    files: &[String],
    context: &ToolContext,
) -> Result<DecisionRequest, String> {
    if files.is_empty() {
        return Ok(request);
    }
    if files.len() > MAX_FILES {
        return Err(format!(
            "too many files: {} (limit {MAX_FILES})",
            files.len()
        ));
    }
    let store = context
        .file_store
        .as_ref()
        .ok_or("Attaching files needs a session file store")?;
    let mut total = 0usize;
    for path in files {
        let mime = media_type(path).ok_or_else(|| {
            format!("{path}: decision models read images, audio, video and PDFs only")
        })?;
        let file = store
            .read_file(context.session_id, &store_path(path))
            .await
            .map_err(|_| format!("{path}: could not be read"))?
            .ok_or_else(|| format!("{path}: no such session file"))?;
        if file.is_directory {
            return Err(format!("{path}: is a directory"));
        }
        let content = file
            .content
            .as_deref()
            .ok_or_else(|| format!("{path}: has no content"))?;
        let bytes = SessionFile::decode_content(content, &file.encoding)
            .map_err(|_| format!("{path}: could not be decoded"))?;
        total = total.saturating_add(bytes.len());
        if total > MAX_TOTAL_BYTES {
            return Err(format!(
                "attached files exceed {} MiB in total",
                MAX_TOTAL_BYTES / (1024 * 1024)
            ));
        }
        request = request.attach(DecisionMedia::bytes(mime, bytes));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_map_to_store_paths_and_media_types() {
        assert_eq!(store_path("/workspace/in/a.png"), "/in/a.png");
        assert_eq!(store_path("a.png"), "/a.png");
        assert_eq!(store_path("/a.png"), "/a.png");
        assert_eq!(media_type("/x/Photo.JPG"), Some("image/jpeg"));
        assert_eq!(media_type("call.m4a"), Some("audio/mp4"));
        assert_eq!(media_type("invoice.pdf"), Some("application/pdf"));
        assert_eq!(media_type("notes.md"), None);
        assert_eq!(media_type("no-extension"), None);
    }
}
