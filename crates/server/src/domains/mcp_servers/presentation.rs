//! Display metadata discovered from a remote MCP server.
//!
//! The operator's `name` and `description` stay the catalog identity. This
//! record is what the server itself publishes: the public server card
//! (`/.well-known/mcp/server-card.json`), RFC 9728 `resource_name`, and
//! `serverInfo` from a later handshake. Icons are same-origin `https` images
//! or small raster `data:` URIs. SVG and any other scheme are dropped
//! (TM-TOOL-062).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use everruns_contracts::url_validation::validate_safe_url;
use everruns_core::{EgressRequest, EgressRequestKind, EgressService};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::storage::{McpServerRow, StorageBackend};

const DOCUMENT_BYTES: usize = 256 * 1024;
const DATA_URI_BYTES: usize = 32 * 1024;
const MAX_ICONS: usize = 4;
const MAX_TITLE: usize = 120;
const MAX_DESCRIPTION: usize = 500;
const MAX_VERSION: usize = 64;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a stored record is reused before we ask the server again.
const FRESH_FOR: Duration = Duration::from_secs(24 * 60 * 60);
/// Suppresses repeat fetches after a transport failure.
const FAILURE_FOR: Duration = Duration::from_secs(5 * 60);
const REFRESH_PARALLEL: usize = 8;

static ATTEMPTS: Mutex<Vec<(i64, Uuid, Instant)>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum McpServerIconTheme {
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct McpServerIcon {
    /// `https` URL on the MCP server's origin, or a raster `data:` URI.
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<McpServerIconTheme>,
}

/// What the remote server says about itself. Absent fields were not published.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, Default)]
pub struct McpServerPresentation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "GitHub MCP Server")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://github.com")]
    pub website_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<McpServerIcon>,
    /// Which documents contributed. Not shown in the UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<DateTime<Utc>>,
}

impl McpServerPresentation {
    pub fn from_value(value: &Value) -> Option<Self> {
        if value.is_null() || value.as_object().is_some_and(|object| object.is_empty()) {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }

    /// True when there is nothing to render. A fetch that found nothing still
    /// has `fetched_at`, and that stays out of the API.
    pub fn has_display(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.website_url.is_some()
            || self.documentation_url.is_some()
            || self.version.is_some()
            || !self.icons.is_empty()
    }

    pub fn for_api(value: &Value) -> Option<Self> {
        Self::from_value(value).filter(Self::has_display)
    }

    /// Theme-neutral icon: one with no theme, otherwise the first.
    pub fn icon_src(&self) -> Option<&str> {
        self.icons
            .iter()
            .find(|icon| icon.theme.is_none())
            .or_else(|| self.icons.first())
            .map(|icon| icon.src.as_str())
    }
}

/// Read the public server card and protected-resource metadata for `server_url`.
///
/// A transport failure leaves `reached` false so the caller can try again
/// later. A 404 is a reached server that published nothing.
pub async fn discover(egress: &dyn EgressService, server_url: &str) -> McpServerPresentation {
    let Ok(server) = validate_safe_url(server_url) else {
        return stamp(McpServerPresentation::default());
    };
    let mut reached = false;
    let mut presentation = McpServerPresentation::default();

    if let Some(document) = get_json(egress, &protected_resource_url(&server)).await {
        reached = true;
        if let Some(title) = document
            .get("resource_name")
            .and_then(Value::as_str)
            .and_then(|text| clean_text(text, MAX_TITLE))
        {
            presentation.title = Some(title);
            push_source(&mut presentation, "resource_metadata");
        }
    }
    if let Some(document) = get_json(egress, &server_card_url(&server)).await {
        reached = true;
        merge_into(
            &mut presentation,
            from_server_info(document.get("serverInfo").unwrap_or(&document), &server),
            "server_card",
        );
    }

    if reached {
        stamp(presentation)
    } else {
        // No `fetched_at`: the caller does not persist a miss, and a short
        // in-process suppress keeps the next page load from retrying.
        presentation
    }
}

/// `serverInfo` from a handshake or `tools/list`, sanitized and folded into
/// whatever is already stored. Best-effort: a write failure does not fail the
/// tool call that observed it.
pub async fn merge_server_info(
    db: &StorageBackend,
    org_id: i64,
    id: Uuid,
    server_url: &str,
    server_info: &Value,
) {
    let Ok(server) = validate_safe_url(server_url) else {
        return;
    };
    let incoming = from_server_info(server_info, &server);
    if !incoming.has_display() {
        return;
    }
    let Ok(Some(owned)) = db.get_mcp_server_with_owner(org_id, id).await else {
        return;
    };
    let mut base = McpServerPresentation::from_value(&owned.row.presentation).unwrap_or_default();
    let before = base.clone();
    merge_into(&mut base, incoming, "server_info");
    if base.title == before.title
        && base.description == before.description
        && base.website_url == before.website_url
        && base.documentation_url == before.documentation_url
        && base.version == before.version
        && base.icons == before.icons
    {
        return;
    }
    let Ok(value) = serde_json::to_value(stamp(base)) else {
        return;
    };
    let _ = db.set_mcp_server_presentation(org_id, id, value).await;
}

/// Fetch documents for every stale row and store what came back.
///
/// Rows that were refreshed are updated in place. A transport failure is
/// remembered for a few minutes so a list page does not fan out again.
pub async fn refresh_rows(
    db: &StorageBackend,
    egress: &dyn EgressService,
    org_id: i64,
    rows: &mut [McpServerRow],
) {
    let jobs: Vec<(Uuid, String)> = rows
        .iter()
        .filter(|row| stale(&row.presentation) && claim(org_id, row.id.uuid()))
        .map(|row| (row.id.uuid(), row.url.clone()))
        .collect();
    if jobs.is_empty() {
        return;
    }
    let mut found = Vec::with_capacity(jobs.len());
    for chunk in jobs.chunks(REFRESH_PARALLEL) {
        found.extend(
            join_all(chunk.iter().map(|(id, url)| {
                let url = url.clone();
                let id = *id;
                async move {
                    let presentation = discover(egress, &url).await;
                    (id, presentation)
                }
            }))
            .await,
        );
    }
    for (id, presentation) in found {
        if presentation.fetched_at.is_none() {
            continue;
        }
        let Ok(value) = serde_json::to_value(&presentation) else {
            continue;
        };
        if db
            .set_mcp_server_presentation(org_id, id, value.clone())
            .await
            .is_err()
        {
            continue;
        }
        if let Some(row) = rows.iter_mut().find(|row| row.id.uuid() == id) {
            row.presentation = value;
        }
    }
}

/// Discover and store for one server, ignoring the freshness window.
pub async fn discover_now(
    db: &StorageBackend,
    egress: &dyn EgressService,
    org_id: i64,
    id: Uuid,
    server_url: &str,
) -> Value {
    let _ = claim(org_id, id);
    let presentation = discover(egress, server_url).await;
    let value = serde_json::to_value(&presentation).unwrap_or_else(|_| json!({}));
    if presentation.fetched_at.is_some() {
        let _ = db
            .set_mcp_server_presentation(org_id, id, value.clone())
            .await;
    }
    value
}

fn stamp(mut presentation: McpServerPresentation) -> McpServerPresentation {
    presentation.fetched_at = Some(Utc::now());
    presentation
}

fn stale(value: &Value) -> bool {
    let Some(fetched) = value
        .get("fetched_at")
        .and_then(Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
    else {
        return true;
    };
    Utc::now().signed_duration_since(fetched)
        > chrono::Duration::from_std(FRESH_FOR).unwrap_or_default()
}

fn claim(org_id: i64, id: Uuid) -> bool {
    let mut attempts = ATTEMPTS.lock().unwrap_or_else(|poison| poison.into_inner());
    let now = Instant::now();
    attempts.retain(|(_, _, at)| now.saturating_duration_since(*at) < FAILURE_FOR);
    if attempts
        .iter()
        .any(|(org, server, _)| *org == org_id && *server == id)
    {
        return false;
    }
    attempts.push((org_id, id, now));
    true
}

fn merge_into(base: &mut McpServerPresentation, incoming: McpServerPresentation, source: &str) {
    let displayed = incoming.has_display();
    if incoming.title.is_some() {
        base.title = incoming.title;
    }
    if incoming.description.is_some() {
        base.description = incoming.description;
    }
    if incoming.website_url.is_some() {
        base.website_url = incoming.website_url;
    }
    if incoming.documentation_url.is_some() {
        base.documentation_url = incoming.documentation_url;
    }
    if incoming.version.is_some() {
        base.version = incoming.version;
    }
    if !incoming.icons.is_empty() {
        base.icons = incoming.icons;
    }
    if displayed {
        push_source(base, source);
    }
}

fn push_source(presentation: &mut McpServerPresentation, source: &str) {
    if !presentation
        .sources
        .iter()
        .any(|existing| existing == source)
    {
        presentation.sources.push(source.to_string());
    }
}

fn from_server_info(value: &Value, server: &Url) -> McpServerPresentation {
    let mut presentation = McpServerPresentation::default();
    presentation.title = value
        .get("title")
        .and_then(Value::as_str)
        .and_then(|text| clean_text(text, MAX_TITLE));
    presentation.description = value
        .get("description")
        .and_then(Value::as_str)
        .and_then(|text| clean_text(text, MAX_DESCRIPTION));
    presentation.website_url = value
        .get("websiteUrl")
        .and_then(Value::as_str)
        .and_then(clean_link);
    presentation.documentation_url = value
        .get("documentationUrl")
        .and_then(Value::as_str)
        .and_then(clean_link);
    presentation.version = value
        .get("version")
        .and_then(Value::as_str)
        .and_then(|text| clean_text(text, MAX_VERSION));
    presentation.icons = value
        .get("icons")
        .and_then(Value::as_array)
        .map(|icons| sanitize_icons(icons, server))
        .unwrap_or_default();
    presentation
}

/// THREAT[TM-TOOL-062]: a remote server's icon is rendered in product chrome.
/// Only same-origin `https` (or `http` when the server itself is `http`) and
/// small raster `data:` URIs survive. SVG can carry script, so it is dropped
/// even inside an `<img>`.
fn sanitize_icons(icons: &[Value], server: &Url) -> Vec<McpServerIcon> {
    icons
        .iter()
        .filter_map(|icon| sanitize_icon(icon, server))
        .take(MAX_ICONS)
        .collect()
}

fn sanitize_icon(icon: &Value, server: &Url) -> Option<McpServerIcon> {
    let src = icon.get("src").and_then(Value::as_str)?;
    let mime = icon
        .get("mimeType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|mime| !mime.is_empty());
    if mime.is_some_and(is_svg) || src.to_ascii_lowercase().contains("image/svg") {
        return None;
    }
    let (src, mime) = if let Some(rest) = src.strip_prefix("data:") {
        parse_data_icon(rest, mime)?
    } else {
        let url = validate_safe_url(src).ok()?;
        if !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        if url.origin() != server.origin() {
            return None;
        }
        if url.scheme() != server.scheme() {
            return None;
        }
        if mime.as_deref().is_some_and(|mime| !is_raster(mime)) {
            return None;
        }
        (url.to_string(), mime.map(str::to_string))
    };
    let sizes = icon
        .get("sizes")
        .and_then(Value::as_array)
        .map(|sizes| {
            sizes
                .iter()
                .filter_map(Value::as_str)
                .filter(|size| valid_size(size))
                .take(4)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let theme = match icon.get("theme").and_then(Value::as_str) {
        Some("light") => Some(McpServerIconTheme::Light),
        Some("dark") => Some(McpServerIconTheme::Dark),
        _ => None,
    };
    Some(McpServerIcon {
        src,
        mime_type: mime,
        sizes,
        theme,
    })
}

fn parse_data_icon(rest: &str, declared: Option<&str>) -> Option<(String, Option<String>)> {
    let (meta, data) = rest.split_once(',')?;
    if data.len() > DATA_URI_BYTES || !meta.ends_with(";base64") {
        return None;
    }
    let mime = meta.trim_end_matches(";base64").trim();
    if !is_raster(mime) || declared.is_some_and(|declared| declared != mime) {
        return None;
    }
    if !data
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
    {
        return None;
    }
    Some((format!("data:{meta},{data}"), Some(mime.to_string())))
}

fn is_raster(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/jpg" | "image/webp" | "image/gif"
    )
}

fn is_svg(mime: &str) -> bool {
    mime.eq_ignore_ascii_case("image/svg+xml")
}

fn valid_size(size: &str) -> bool {
    if size == "any" {
        return true;
    }
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    let number = |part: &str| {
        !part.is_empty()
            && part.len() <= 4
            && !part.starts_with('0')
            && part.chars().all(|c| c.is_ascii_digit())
    };
    number(width) && number(height)
}

fn clean_text(value: &str, max: usize) -> Option<String> {
    let text: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn clean_link(value: &str) -> Option<String> {
    let url = validate_safe_url(value.trim()).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(url.to_string())
}

fn server_card_url(server: &Url) -> String {
    format!("{}/.well-known/mcp/server-card.json", origin_of(server))
}

fn protected_resource_url(server: &Url) -> String {
    let origin = origin_of(server);
    let path = server.path().trim_end_matches('/');
    if path.is_empty() {
        format!("{origin}/.well-known/oauth-protected-resource")
    } else {
        format!("{origin}/.well-known/oauth-protected-resource{path}")
    }
}

fn origin_of(url: &Url) -> String {
    format!(
        "{}://{}{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    )
}

async fn get_json(egress: &dyn EgressService, url: &str) -> Option<Value> {
    if validate_safe_url(url).is_err() {
        return None;
    }
    let request = EgressRequest::new("GET", url, EgressRequestKind::Mcp)
        .header("accept", "application/json")
        .timeout_ms(REQUEST_TIMEOUT.as_millis() as u64)
        .require_dns_pinning();
    let response = egress.send(request).await.ok()?;
    if response.status != 200 || response.body.len() > DOCUMENT_BYTES {
        return None;
    }
    serde_json::from_slice(&response.body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::{EgressError, EgressResponse, EgressResult, EgressStreamResponse};
    use std::collections::HashMap;

    struct MapEgress {
        routes: HashMap<String, (u16, &'static str)>,
    }

    #[async_trait]
    impl EgressService for MapEgress {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            let (status, body) = self.routes.get(&request.url).copied().unwrap_or((404, ""));
            Ok(EgressResponse {
                status,
                headers: Default::default(),
                body: body.as_bytes().to_vec(),
            })
        }

        async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            Err(EgressError::Transport("unused".into()))
        }
    }

    fn github() -> Url {
        Url::parse("https://api.githubcopilot.com/mcp/").unwrap()
    }

    #[test]
    fn resource_name_and_server_card_merge_and_icons_stay_on_the_server_origin() {
        let server = github();
        let mut presentation = McpServerPresentation {
            title: Some("GitHub MCP Server".into()),
            sources: vec!["resource_metadata".into()],
            ..Default::default()
        };
        let card = json!({
            "serverInfo": {
                "name": "github-mcp-server",
                "title": "GitHub",
                "description": "Official GitHub MCP server",
                "websiteUrl": "https://github.com",
                "documentationUrl": "javascript:alert(1)",
                "version": "1.2.3",
                "icons": [
                    {"src": "https://api.githubcopilot.com/icon.png", "mimeType": "image/png", "sizes": ["48x48"], "theme": "light"},
                    {"src": "https://evil.example/icon.png", "mimeType": "image/png"},
                    {"src": "https://api.githubcopilot.com/icon.svg", "mimeType": "image/svg+xml"},
                    {"src": "data:image/png;base64,aaaa", "mimeType": "image/png"}
                ]
            }
        });
        merge_into(
            &mut presentation,
            from_server_info(&card["serverInfo"], &server),
            "server_card",
        );
        assert_eq!(presentation.title.as_deref(), Some("GitHub"));
        assert_eq!(
            presentation.website_url.as_deref(),
            Some("https://github.com/")
        );
        assert!(presentation.documentation_url.is_none());
        assert_eq!(presentation.icons.len(), 2);
        assert_eq!(
            presentation.icons[0].src,
            "https://api.githubcopilot.com/icon.png"
        );
        assert_eq!(presentation.icons[0].theme, Some(McpServerIconTheme::Light));
        assert!(presentation.icons[1].src.starts_with("data:image/png"));
        assert_eq!(
            presentation.sources,
            vec!["resource_metadata".to_string(), "server_card".to_string()]
        );
    }

    #[test]
    fn protected_resource_url_inserts_well_known_before_the_path() {
        let server = github();
        assert_eq!(
            protected_resource_url(&server),
            "https://api.githubcopilot.com/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            server_card_url(&server),
            "https://api.githubcopilot.com/.well-known/mcp/server-card.json"
        );
    }

    #[tokio::test]
    async fn discover_reads_the_card_and_the_resource_name() {
        let egress = MapEgress {
            routes: HashMap::from([
                (
                    "https://mcp.linear.app/.well-known/oauth-protected-resource/mcp".to_string(),
                    (200, r#"{"resource":"https://mcp.linear.app/mcp"}"#),
                ),
                (
                    "https://mcp.linear.app/.well-known/mcp/server-card.json".to_string(),
                    (
                        200,
                        r#"{"serverInfo":{"title":"Linear","websiteUrl":"https://linear.app","icons":[{"src":"https://mcp.linear.app/icon.png","mimeType":"image/png"}]}}"#,
                    ),
                ),
            ]),
        };
        let presentation = discover(&egress, "https://mcp.linear.app/mcp").await;
        assert_eq!(presentation.title.as_deref(), Some("Linear"));
        assert_eq!(presentation.icons.len(), 1);
        assert!(presentation.fetched_at.is_some());
        assert_eq!(presentation.sources, vec!["server_card".to_string()]);
    }

    #[tokio::test]
    async fn discover_uses_resource_name_when_the_card_is_missing() {
        let egress = MapEgress {
            routes: HashMap::from([(
                "https://api.githubcopilot.com/.well-known/oauth-protected-resource/mcp"
                    .to_string(),
                (
                    200,
                    r#"{"resource_name":"GitHub MCP Server","resource":"https://api.githubcopilot.com/mcp/"}"#,
                ),
            )]),
        };
        let presentation = discover(&egress, "https://api.githubcopilot.com/mcp/").await;
        assert_eq!(presentation.title.as_deref(), Some("GitHub MCP Server"));
        assert!(presentation.icons.is_empty());
        assert_eq!(presentation.sources, vec!["resource_metadata".to_string()]);
    }
}
