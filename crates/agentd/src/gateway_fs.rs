//! A session's workspace on a data plane (step 11.1): the control plane's
//! file API (`/v1/workspaces/{id}/fs`, and the older
//! `/v1/sessions/{id}/fs` aliases) in its shapes, so the console's file
//! panel, uploads and downloads, and the CLI work unchanged for a v2 org.
//!
//! Decisions:
//! - A session's workspace has the session's id (`wsp_` and `session_`
//!   name the same UUID), as on the control plane, where every session's
//!   default workspace id equals its session id. A shard has no shared
//!   workspaces.
//! - Each call is one [`FsOp`] on the session's shard, through the same
//!   retry loop as the other session commands (moves, freezes, a leader
//!   change). Reads are linearizable there (`api_files.rs`).
//! - Downloads are served whole from one read: files are bounded by
//!   [`FILE_MAX`], so `Range` is answered from memory, which is what a
//!   `<video>` seeking a recording needs.
//! - Uploads take raw bytes: UTF-8 is stored as text, anything else as
//!   base64, as the control plane stores them.
//! - The HTML preview keeps the control plane's sandbox (TM-WEB-010): an
//!   opaque origin with scripts but no network, so a previewed page cannot
//!   read the console's cookies or send the document anywhere.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use everruns_contracts::typed_id::{SessionId, WorkspaceId};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::api_files::{FILE_MAX, FsOp, FsResult};
use crate::gateway::Gateway;

type ApiError = (StatusCode, String);

/// JSON calls carry at most this much (base64 content included), as on the
/// control plane.
const JSON_MAX: usize = 10 * 1024 * 1024;

/// The workspace routes under `base` (`/v1/workspaces/{session}` or
/// `/v1/sessions/{session}`; one parameter name, as the router requires).
fn routes_under(base: &str) -> Router<Arc<Gateway>> {
    Router::new()
        .route(&format!("{base}/fs/_/move"), post(move_file))
        .route(&format!("{base}/fs/_/copy"), post(copy_file))
        .route(&format!("{base}/fs/_/grep"), post(grep))
        .route(&format!("{base}/fs/_/search"), post(search))
        .route(&format!("{base}/fs/_/stat"), post(stat))
        .route(&format!("{base}/fs/_/download/{{*path}}"), get(download))
        .route(&format!("{base}/fs/_/preview/{{*path}}"), get(preview))
        .route(
            &format!("{base}/fs"),
            get(get_root).post(create_root).delete(delete_root),
        )
        .route(
            &format!("{base}/fs/{{*path}}"),
            get(get_path)
                .post(create_path)
                .put(update_path)
                .delete(delete_path),
        )
        .layer(DefaultBodyLimit::max(JSON_MAX))
        .route(
            &format!("{base}/fs/_/upload/{{*path}}"),
            put(upload).layer(DefaultBodyLimit::max(FILE_MAX)),
        )
}

pub fn routes() -> Router<Arc<Gateway>> {
    routes_under("/v1/workspaces/{session}").merge(routes_under("/v1/sessions/{session}"))
}

/// The session a workspace id (or a session id) names.
pub(crate) fn session_of(id: &str) -> Result<SessionId, ApiError> {
    if let Ok(workspace) = WorkspaceId::parse(id) {
        return Ok(SessionId::from_uuid(workspace.uuid()));
    }
    SessionId::parse(id).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

/// Run `op` on the session's shard.
async fn call(gateway: &Gateway, id: &str, op: FsOp) -> Result<Value, ApiError> {
    let session = session_of(id)?;
    let body = serde_json::to_value(&op)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let value = gateway.session_command(session, "fs", &body).await?;
    match serde_json::from_value::<FsResult>(value) {
        Ok(FsResult::Done { value }) => Ok(value),
        Ok(FsResult::Failed { status, message }) => Err((
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            message,
        )),
        Err(error) => Err((StatusCode::BAD_GATEWAY, error.to_string())),
    }
}

#[derive(Deserialize, Default)]
struct Recursive {
    #[serde(default)]
    recursive: bool,
}

async fn get_root(
    State(gateway): State<Arc<Gateway>>,
    Path(id): Path<String>,
    Query(query): Query<Recursive>,
) -> Result<Json<Value>, ApiError> {
    let op = FsOp::Get {
        path: "/".into(),
        recursive: query.recursive,
    };
    Ok(Json(call(&gateway, &id, op).await?))
}

async fn get_path(
    State(gateway): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<Recursive>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if wants_raw(&headers) {
        return raw(&gateway, &id, path, &headers, false).await;
    }
    let op = FsOp::Get {
        path,
        recursive: query.recursive,
    };
    Ok(Json(call(&gateway, &id, op).await?).into_response())
}

async fn create_root() -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        "Cannot create at root path, specify a path".into(),
    )
}

async fn delete_root() -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        "Cannot delete root directory".into(),
    )
}

#[derive(Deserialize)]
struct CreateBody {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    is_readonly: Option<bool>,
    #[serde(default)]
    is_directory: Option<bool>,
}

async fn create_path(
    State(gateway): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let op = FsOp::Create {
        path,
        content: body.content,
        encoding: body.encoding,
        readonly: body.is_readonly.unwrap_or(false),
        directory: body.is_directory.unwrap_or(false),
    };
    Ok((StatusCode::CREATED, Json(call(&gateway, &id, op).await?)))
}

#[derive(Deserialize)]
struct UpdateBody {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    is_readonly: Option<bool>,
    #[serde(default)]
    expected_content: Option<String>,
    #[serde(default)]
    expected_encoding: Option<String>,
}

async fn update_path(
    State(gateway): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    Json(body): Json<UpdateBody>,
) -> Result<Json<Value>, ApiError> {
    let op = FsOp::Update {
        path,
        content: body.content,
        encoding: body.encoding,
        readonly: body.is_readonly,
        expected_content: body.expected_content,
        expected_encoding: body.expected_encoding,
    };
    Ok(Json(call(&gateway, &id, op).await?))
}

async fn delete_path(
    State(gateway): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<Recursive>,
) -> Result<Json<Value>, ApiError> {
    let op = FsOp::Delete {
        path,
        recursive: query.recursive,
    };
    Ok(Json(call(&gateway, &id, op).await?))
}

#[derive(Deserialize)]
struct SrcDst {
    src_path: String,
    dst_path: String,
}

async fn move_file(
    State(gateway): State<Arc<Gateway>>,
    Path(id): Path<String>,
    Json(body): Json<SrcDst>,
) -> Result<Json<Value>, ApiError> {
    let op = FsOp::Move {
        src: body.src_path,
        dst: body.dst_path,
    };
    Ok(Json(call(&gateway, &id, op).await?))
}

async fn copy_file(
    State(gateway): State<Arc<Gateway>>,
    Path(id): Path<String>,
    Json(body): Json<SrcDst>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let op = FsOp::Copy {
        src: body.src_path,
        dst: body.dst_path,
    };
    Ok((StatusCode::CREATED, Json(call(&gateway, &id, op).await?)))
}

#[derive(Deserialize)]
struct GrepBody {
    pattern: String,
    #[serde(default)]
    path_pattern: Option<String>,
}

async fn grep(
    State(gateway): State<Arc<Gateway>>,
    Path(id): Path<String>,
    Json(body): Json<GrepBody>,
) -> Result<Json<Value>, ApiError> {
    let op = FsOp::Grep {
        pattern: body.pattern,
        path_pattern: body.path_pattern,
    };
    Ok(Json(call(&gateway, &id, op).await?))
}

#[derive(Deserialize)]
struct SearchBody {
    pattern: String,
    #[serde(default)]
    path_pattern: Option<String>,
    #[serde(default)]
    before_context: usize,
    #[serde(default)]
    after_context: usize,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    max_bytes: Option<usize>,
}

async fn search(
    State(gateway): State<Arc<Gateway>>,
    Path(id): Path<String>,
    Json(body): Json<SearchBody>,
) -> Result<Json<Value>, ApiError> {
    let op = FsOp::Search {
        pattern: body.pattern,
        path_pattern: body.path_pattern,
        before_context: body.before_context,
        after_context: body.after_context,
        offset: body.offset,
        limit: body.limit,
        max_bytes: body.max_bytes,
    };
    Ok(Json(call(&gateway, &id, op).await?))
}

#[derive(Deserialize)]
struct StatBody {
    path: String,
}

async fn stat(
    State(gateway): State<Arc<Gateway>>,
    Path(id): Path<String>,
    Json(body): Json<StatBody>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        call(&gateway, &id, FsOp::Stat { path: body.path }).await?,
    ))
}

async fn upload(
    State(gateway): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let size = body.len() as u64;
    let (content, encoding) = match String::from_utf8(body.to_vec()) {
        Ok(text) => (text, "text"),
        Err(_) => (
            base64::engine::general_purpose::STANDARD.encode(&body),
            "base64",
        ),
    };
    let op = FsOp::Upload {
        path,
        content,
        encoding: encoding.into(),
        size,
    };
    Ok(Json(call(&gateway, &id, op).await?))
}

async fn download(
    State(gateway): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    raw(&gateway, &id, path, &headers, true).await
}

/// A file's bytes and its name.
async fn bytes_of(
    gateway: &Gateway,
    id: &str,
    path: String,
) -> Result<(Vec<u8>, String), ApiError> {
    let value = call(gateway, id, FsOp::Raw { path }).await?;
    let content = value["content"].as_str().unwrap_or_default();
    let bytes = match value["encoding"].as_str() {
        Some("base64") => base64::engine::general_purpose::STANDARD
            .decode(content)
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to decode file content".to_owned(),
                )
            })?,
        _ => content.as_bytes().to_vec(),
    };
    Ok((
        bytes,
        value["name"].as_str().unwrap_or("download").to_owned(),
    ))
}

async fn raw(
    gateway: &Gateway,
    id: &str,
    path: String,
    headers: &HeaderMap,
    attachment: bool,
) -> Result<Response, ApiError> {
    let content_type = content_type(&path);
    let (bytes, name) = bytes_of(gateway, id, path).await?;
    Ok(ranged(bytes, &name, content_type, headers, attachment))
}

/// The content type a path's extension names, else text or bytes.
fn content_type(path: &str) -> Option<String> {
    mime_guess::from_path(path)
        .first_raw()
        .map(ToOwned::to_owned)
}

/// A byte range a request asks for, resolved against `size`: `Ok(None)` for
/// the whole file, `Err` when it lies outside.
fn requested(headers: &HeaderMap, size: u64, etag: &str) -> Result<Option<(u64, u64)>, ()> {
    let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    let fresh = headers
        .get(header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|validator| validator.trim() == etag);
    if !fresh {
        return Ok(None);
    }
    let Some(spec) = range.trim().strip_prefix("bytes=") else {
        return Ok(None);
    };
    // One range only; a list is answered with the whole file.
    if spec.contains(',') {
        return Ok(None);
    }
    let Some((first, last)) = spec.split_once('-') else {
        return Ok(None);
    };
    let (start, end) = match (first.trim(), last.trim()) {
        ("", "") => return Ok(None),
        ("", suffix) => {
            let n: u64 = suffix.parse().map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            (size.saturating_sub(n), size)
        }
        (first, "") => (first.parse().map_err(|_| ())?, size),
        (first, last) => {
            let first: u64 = first.parse().map_err(|_| ())?;
            let last: u64 = last.parse().map_err(|_| ())?;
            if last < first {
                return Err(());
            }
            (first, (last + 1).min(size))
        }
    };
    if start >= size {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn ranged(
    bytes: Vec<u8>,
    name: &str,
    content_type: Option<String>,
    headers: &HeaderMap,
    attachment: bool,
) -> Response {
    let size = bytes.len() as u64;
    let etag = format!("\"{}\"", hex::encode(Sha256::digest(&bytes)));
    let content_type = content_type.unwrap_or_else(|| {
        if std::str::from_utf8(&bytes).is_ok() {
            "text/plain; charset=utf-8".into()
        } else {
            "application/octet-stream".into()
        }
    });
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, &etag)
        .header(header::CONTENT_DISPOSITION, disposition(name, attachment));
    let response = match requested(headers, size, &etag) {
        Err(()) => Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{size}"))
            .header(header::ACCEPT_RANGES, "bytes")
            .body(Body::empty()),
        Ok(Some((start, end))) => builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{size}", end - 1),
            )
            .body(Body::from(bytes[start as usize..end as usize].to_vec())),
        Ok(None) => builder.status(StatusCode::OK).body(Body::from(bytes)),
    };
    response.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `Content-Disposition` with an ASCII fallback and the UTF-8 name.
fn disposition(name: &str, attachment: bool) -> String {
    let kind = if attachment { "attachment" } else { "inline" };
    let fallback: String = name
        .chars()
        .map(|c| match c {
            '"' | '\\' | '/' | ';' => '_',
            ' '..='~' => c,
            _ => '_',
        })
        .collect();
    let fallback = match fallback.trim() {
        "" => "download".to_owned(),
        trimmed => trimmed.to_owned(),
    };
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// Whether the client asked for raw bytes (`Accept: application/octet-stream`
/// with a positive quality), as the control plane decides it.
fn wants_raw(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| {
            accept.split(',').any(|entry| {
                let mut parts = entry.split(';');
                let media = parts.next().unwrap_or_default().trim();
                let quality = parts
                    .filter_map(|p| p.trim().strip_prefix("q="))
                    .next()
                    .and_then(|q| q.parse::<f32>().ok())
                    .unwrap_or(1.0);
                media.eq_ignore_ascii_case("application/octet-stream") && quality > 0.0
            })
        })
}

/// The control plane's preview sandbox (TM-WEB-010), verbatim.
const PREVIEW_CSP: &str = "sandbox allow-scripts; default-src 'none'; \
script-src 'unsafe-inline' 'unsafe-eval' data: blob:; \
style-src 'unsafe-inline' data: blob:; img-src data: blob:; \
font-src data: blob:; media-src data: blob:; connect-src 'none'; \
object-src 'none'; base-uri 'none'";

async fn preview(
    State(gateway): State<Arc<Gateway>>,
    Path((id, path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".html") || lower.ends_with(".htm")) {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Only HTML files can be previewed".into(),
        ));
    }
    let (bytes, _) = bytes_of(&gateway, &id, path).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CONTENT_SECURITY_POLICY, PREVIEW_CSP)
        .header(header::X_FRAME_OPTIONS, "SAMEORIGIN")
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(name: &str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        headers
    }

    #[test]
    fn ranges_resolve_like_the_control_plane() {
        let etag = "\"e\"";
        assert_eq!(requested(&HeaderMap::new(), 10, etag), Ok(None));
        assert_eq!(
            requested(&with("range", "bytes=0-3"), 10, etag),
            Ok(Some((0, 4)))
        );
        assert_eq!(
            requested(&with("range", "bytes=7-"), 10, etag),
            Ok(Some((7, 10)))
        );
        assert_eq!(
            requested(&with("range", "bytes=-4"), 10, etag),
            Ok(Some((6, 10)))
        );
        assert_eq!(
            requested(&with("range", "bytes=5-99"), 10, etag),
            Ok(Some((5, 10)))
        );
        assert_eq!(requested(&with("range", "bytes=10-"), 10, etag), Err(()));
        assert_eq!(
            requested(&with("range", "bytes=0-1,4-5"), 10, etag),
            Ok(None)
        );
    }

    #[test]
    fn a_stale_if_range_gets_the_whole_file() {
        let mut headers = with("range", "bytes=0-3");
        headers.insert(header::IF_RANGE, "\"old\"".parse().unwrap());
        assert_eq!(requested(&headers, 10, "\"new\""), Ok(None));
    }

    #[test]
    fn a_workspace_id_names_its_session() {
        let session = SessionId::new();
        let workspace = WorkspaceId::from_uuid(session.uuid());
        assert_eq!(session_of(&workspace.to_string()).unwrap(), session);
        assert_eq!(session_of(&session.to_string()).unwrap(), session);
        assert!(session_of("wsp_nope").is_err());
    }

    #[test]
    fn raw_bytes_need_a_positive_quality() {
        assert!(wants_raw(&with("accept", "application/octet-stream")));
        assert!(!wants_raw(&with(
            "accept",
            "application/octet-stream;q=0, application/json"
        )));
        assert!(!wants_raw(&with("accept", "application/json")));
    }

    #[test]
    fn a_disposition_keeps_an_ascii_fallback() {
        let value = disposition("report \"Δ\".pdf", true);
        assert!(value.starts_with("attachment; "));
        assert!(value.contains("filename=\"report ___.pdf\""));
        assert!(value.contains("filename*=UTF-8''report%20%22%CE%94%22.pdf"));
    }
}
