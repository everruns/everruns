// MCP Apps host endpoints: what a view in the console needs from the server.
//
// Decisions:
// - The view's HTML is read from its MCP server when someone opens the chat,
//   never stored on the event (views are tens of KB and change with the
//   server). Reading uses the session's own MCP connection, so it reaches only
//   servers the session already has and runs through the org's egress policy.
// - Anyone who can view the session can load a view. Calls a view makes
//   (`tools/call`, `resources/read`) run with the session's MCP credentials,
//   so only the session's owner may make them; everyone else gets the view
//   with the data the tool call already returned.
// - A view may call a tool without asking when the server marks it read-only
//   (`readOnlyHint`) or app-only (`_meta.ui.visibility` without `model`).
//   Anything else runs only with `confirmed: true`, which the console sends
//   after the person agreed in a dialog. A tool whose visibility leaves out
//   `app` is never callable from a view (SEP-1865).
//
// See knowledge/integrations/mcp-apps-host.md.

use super::SessionCommandService;
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::kernel_imports::Caller;
use anyhow::{Result, anyhow};
use base64::Engine as _;
use everruns_contracts::runtime::mcp_apps::{
    self, MCP_APP_MIME_TYPE, McpAppViewPolicy, McpInteractiveViews,
};
use everruns_contracts::runtime::mcp_resources::McpResourceOutcome;
use everruns_contracts::runtime::mcp_server::{McpToolDefinition, sanitize_mcp_server_name};
use everruns_contracts::typed_id::SessionId;
use everruns_core::McpToolInvoker;
use everruns_core::host::RuntimeHostAdapter;
use everruns_worker::worker_adapters::WorkerAdapters as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use utoipa::ToSchema;

/// Largest view HTML served. Views are single-file bundles; anything bigger is
/// almost certainly not one.
const MAX_VIEW_BYTES: usize = 2 * 1024 * 1024;

/// A view, ready to load in a sandboxed frame.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct McpAppView {
    /// The view's HTML document.
    pub html: String,
    /// Origins the view declared it needs; the frame's CSP allows only these.
    pub policy: McpAppViewPolicyDto,
    /// Where the organization shows this server's views.
    pub interactive_views: McpInteractiveViews,
    /// Whether the caller may let the view call tools and read resources.
    pub can_call_tools: bool,
}

/// Network origins and border preference a view declared.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct McpAppViewPolicyDto {
    pub connect_domains: Vec<String>,
    pub resource_domains: Vec<String>,
    pub frame_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefers_border: Option<bool>,
}

impl From<McpAppViewPolicy> for McpAppViewPolicyDto {
    fn from(policy: McpAppViewPolicy) -> Self {
        Self {
            connect_domains: policy.connect_domains,
            resource_domains: policy.resource_domains,
            frame_domains: policy.frame_domains,
            prefers_border: policy.prefers_border,
        }
    }
}

/// A tool call a view asks the host to make.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct McpAppToolCallRequest {
    /// Sanitized server prefix from the view's tool call (`details.mcp_app.server`).
    pub server: String,
    /// The server's own tool name.
    pub name: String,
    /// Tool arguments.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub arguments: Value,
    /// The person agreed to run a tool that is not read-only.
    #[serde(default)]
    pub confirmed: bool,
}

/// Outcome of a view's tool call.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpAppToolCallResponse {
    /// The tool ran; `result` is an MCP `CallToolResult`.
    Done {
        #[schema(value_type = Object)]
        result: Value,
    },
    /// The tool changes something; ask the person, then retry with `confirmed`.
    ConfirmationRequired {
        /// Tool title to show in the dialog.
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

/// A resource a view asks the host to read.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct McpAppResourceReadRequest {
    /// Sanitized server prefix.
    pub server: String,
    /// Resource URI.
    pub uri: String,
}

/// Whether a view may call `tool` and whether it must ask first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppCallPolicy {
    /// Read-only or app-only: run it.
    Direct,
    /// Changes something: run only after the person confirms.
    Confirm,
    /// The server hid this tool from views.
    Forbidden,
}

pub(crate) fn app_call_policy(tool: &McpToolDefinition) -> AppCallPolicy {
    let visibility = tool
        .meta
        .as_ref()
        .and_then(|meta| meta.get("ui"))
        .and_then(|ui| ui.get("visibility"))
        .and_then(Value::as_array);
    if let Some(visibility) = visibility
        && !visibility.iter().any(|entry| entry.as_str() == Some("app"))
    {
        return AppCallPolicy::Forbidden;
    }
    let read_only = tool
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.read_only_hint)
        .unwrap_or(false);
    if read_only || mcp_apps::tool_is_app_only(tool.meta.as_ref()) {
        AppCallPolicy::Direct
    } else {
        AppCallPolicy::Confirm
    }
}

/// Rebuild the MCP `CallToolResult` a view expects from the internal result.
pub(crate) fn call_tool_result(result: &everruns_contracts::ToolResult) -> Value {
    let mcp_result = result
        .details
        .as_ref()
        .and_then(|details| details.get(mcp_apps::MCP_RESULT_DETAILS_KEY));
    let mut content: Vec<Value> = Vec::new();
    if let Some(error) = &result.error {
        content.push(json!({"type": "text", "text": error}));
    }
    match result.result.as_ref() {
        Some(Value::Object(fields)) if fields.contains_key("content") => {
            if let Some(parts) = fields.get("content").and_then(Value::as_array) {
                content.extend(parts.iter().filter(|part| part["type"] == "text").cloned());
            }
        }
        Some(Value::Object(fields)) => {
            for key in ["result", "error"] {
                if let Some(text) = fields.get(key).and_then(Value::as_str) {
                    content.push(json!({"type": "text", "text": text}));
                }
            }
        }
        Some(other) => content.push(json!({"type": "text", "text": other.to_string()})),
        None => {}
    }
    for image in result.images.iter().flatten() {
        content.push(json!({"type": "image", "data": image.base64, "mimeType": image.media_type}));
    }
    let is_error = result.error.is_some()
        || result
            .result
            .as_ref()
            .is_some_and(|value| value.get("error").is_some());
    let mut out = json!({"content": content, "isError": is_error});
    if let Some(structured) = mcp_result.and_then(|value| value.get("structured_content")) {
        out["structuredContent"] = structured.clone();
    }
    if let Some(meta) = mcp_result.and_then(|value| value.get("meta")) {
        out["_meta"] = meta.clone();
    }
    out
}

impl SessionCommandService {
    /// The session's MCP invoker, as its turns would use it (no input message
    /// bound, so credentials follow the session's owner).
    async fn mcp_app_invoker(
        &self,
        caller: &Caller,
        session_id: SessionId,
    ) -> Result<(Arc<dyn McpToolInvoker>, bool)> {
        let adapters = self
            .adapters()
            .with_egress_service(self.mcp_server_service.egress_service());
        let session = adapters
            .get_stored_session(caller.org_id, session_id.uuid())
            .await?
            .ok_or(ResourceNotFoundError::new("Session"))?;
        super::service::authorize_platform_chat_owner(&self.db, caller, &session).await?;
        let is_owner = caller.is_internal
            || session
                .resolved_owner_user_id
                .is_none_or(|owner| caller.user_id == Some(owner));
        let host = everruns_worker::WorkerRuntimeHost::new(adapters);
        let invoker = host
            .mcp_executor(caller.org_id, session_id, session.agent_id)
            .await
            .ok_or_else(|| anyhow!(BadRequestError::new("This session has no MCP servers")))?;
        Ok((invoker, is_owner))
    }

    /// Where the organization shows views from the server with this prefix.
    /// Servers that are not organization servers (a person's own, a session's)
    /// use the default.
    async fn interactive_views(&self, org_id: i64, server: &str) -> McpInteractiveViews {
        match self.db.list_mcp_servers(org_id, None, false).await {
            Ok(rows) => rows
                .iter()
                .find(|row| sanitize_mcp_server_name(&row.name) == server)
                .map(|row| {
                    crate::domains::mcp_servers::McpServerService::settings_from_row(row)
                        .interactive_views
                })
                .unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, server, "could not read MCP server view setting");
                McpInteractiveViews::default()
            }
        }
    }

    /// Read a view (`ui://`) from one of the session's MCP servers.
    pub async fn read_mcp_app_view(
        &self,
        caller: &Caller,
        session_id: SessionId,
        server: &str,
        uri: &str,
    ) -> Result<McpAppView> {
        if !mcp_apps::is_view_uri(uri) {
            return Err(BadRequestError::new("A view URI must start with ui://").into());
        }
        let interactive_views = self.interactive_views(caller.org_id, server).await;
        if !interactive_views.in_console() {
            return Err(BadRequestError::new("Interactive views are off for this server").into());
        }
        let (invoker, is_owner) = self.mcp_app_invoker(caller, session_id).await?;
        let read = invoker
            .read_resource(server, uri)
            .await
            .map_err(|error| anyhow!(BadRequestError::new(error.to_string())))?
            .ok_or(ResourceNotFoundError::new("MCP server"))?;
        let contents = match read {
            McpResourceOutcome::Done(result) => result.contents,
            McpResourceOutcome::ConnectionRequired(_) => {
                return Err(BadRequestError::new(
                    "Connect this MCP server before its view can load",
                )
                .into());
            }
        };
        let content = contents
            .into_iter()
            .find(|content| {
                content.uri == uri
                    || content
                        .mime_type
                        .as_deref()
                        .is_some_and(|mime| mime.starts_with("text/html"))
            })
            .ok_or(ResourceNotFoundError::new("View"))?;
        if let Some(mime) = content.mime_type.as_deref()
            && !mime.starts_with("text/html")
        {
            return Err(BadRequestError::new(format!(
                "The view is {mime}, not {MCP_APP_MIME_TYPE}"
            ))
            .into());
        }
        let html = match (content.text, content.blob) {
            (Some(text), _) => text,
            (None, Some(blob)) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(blob.as_bytes())
                    .map_err(|_| anyhow!(BadRequestError::new("The view is not valid base64")))?;
                String::from_utf8(bytes)
                    .map_err(|_| anyhow!(BadRequestError::new("The view is not UTF-8")))?
            }
            (None, None) => return Err(ResourceNotFoundError::new("View").into()),
        };
        if html.len() > MAX_VIEW_BYTES {
            return Err(BadRequestError::new("The view is larger than 2 MB").into());
        }
        Ok(McpAppView {
            html,
            policy: McpAppViewPolicy::from_meta(content.meta.as_ref()).into(),
            interactive_views,
            can_call_tools: is_owner,
        })
    }

    /// Run a tool a view asked for.
    pub async fn call_mcp_app_tool(
        &self,
        caller: &Caller,
        session_id: SessionId,
        request: McpAppToolCallRequest,
    ) -> Result<McpAppToolCallResponse> {
        let (invoker, is_owner) = self.mcp_app_invoker(caller, session_id).await?;
        if !is_owner {
            return Err(everruns_core::PolicyError::denied(
                "mcp_app_tool_call",
                "the session owner",
            )
            .into());
        }
        let tools = invoker
            .list_server_tool_specs(&request.server)
            .await
            .map_err(|error| anyhow!(BadRequestError::new(error.to_string())))?
            .ok_or(ResourceNotFoundError::new("MCP server"))?;
        let tool = tools
            .iter()
            .find(|tool| tool.name == request.name)
            .ok_or(ResourceNotFoundError::new("Tool"))?;
        match app_call_policy(tool) {
            AppCallPolicy::Forbidden => {
                return Err(BadRequestError::new(format!(
                    "The server does not let views call {}",
                    request.name
                ))
                .into());
            }
            AppCallPolicy::Confirm if !request.confirmed => {
                return Ok(McpAppToolCallResponse::ConfirmationRequired {
                    title: tool.title.clone().unwrap_or_else(|| tool.name.clone()),
                    description: tool.description.clone(),
                });
            }
            _ => {}
        }
        let arguments = if request.arguments.is_null() {
            json!({})
        } else {
            request.arguments
        };
        let call = everruns_contracts::ToolCall {
            id: format!("app_{}", uuid::Uuid::now_v7()),
            name: format!("mcp_{}__{}", request.server, request.name),
            arguments,
        };
        let result = invoker
            .invoke(&call)
            .await
            .map_err(|error| anyhow!(BadRequestError::new(error.to_string())))?;
        if result.connection_required.is_some() {
            return Err(
                BadRequestError::new("Connect this MCP server before the view can use it").into(),
            );
        }
        Ok(McpAppToolCallResponse::Done {
            result: call_tool_result(&result),
        })
    }

    /// Read a resource a view asked for.
    pub async fn read_mcp_app_resource(
        &self,
        caller: &Caller,
        session_id: SessionId,
        request: McpAppResourceReadRequest,
    ) -> Result<Value> {
        let (invoker, is_owner) = self.mcp_app_invoker(caller, session_id).await?;
        if !is_owner && !mcp_apps::is_view_uri(&request.uri) {
            return Err(everruns_core::PolicyError::denied(
                "mcp_app_resource_read",
                "the session owner",
            )
            .into());
        }
        let read = invoker
            .read_resource(&request.server, &request.uri)
            .await
            .map_err(|error| anyhow!(BadRequestError::new(error.to_string())))?
            .ok_or(ResourceNotFoundError::new("MCP server"))?;
        match read {
            McpResourceOutcome::Done(result) => Ok(serde_json::to_value(result)?),
            McpResourceOutcome::ConnectionRequired(_) => Err(BadRequestError::new(
                "Connect this MCP server before the view can use it",
            )
            .into()),
        }
    }
}

/// What a channel without a browser should show for a view.
#[derive(Debug, Clone, PartialEq)]
pub enum McpAppSnapshot {
    /// The organization turned pictures off for this server: show nothing.
    Off,
    /// No picture could be made (no Browserless connection, or rendering
    /// failed): show the link only.
    LinkOnly,
    /// A PNG of the view as it first rendered.
    Picture(Vec<u8>),
}

/// Width and height of the picture's viewport, in CSS pixels.
const SNAPSHOT_VIEWPORT: (u32, u32) = (720, 480);
/// How long the view gets to draw before the picture is taken.
const SNAPSHOT_SETTLE_MS: u64 = 2_500;

/// A page that hosts the view the way the console does (opaque-origin frame,
/// the view's own CSP, the bridge's initialize/tool-input/tool-result), so a
/// headless browser can take its picture. It answers nothing else: the view
/// cannot call tools, read resources or open links while it is photographed.
pub(crate) fn snapshot_page(view: &McpAppView, tool: &str, tool_result: &Value) -> String {
    let csp = view_csp(&view.policy);
    let html = inject_csp(&view.html, &csp);
    let srcdoc = serde_json::to_string(&html).unwrap_or_else(|_| "\"\"".into());
    let result = serde_json::to_string(tool_result).unwrap_or_else(|_| "{}".into());
    let tool = serde_json::to_string(tool).unwrap_or_else(|_| "\"\"".into());
    let (width, height) = SNAPSHOT_VIEWPORT;
    // `</` cannot appear in the JSON strings above unescaped: serde_json
    // escapes `/` only inside `<\/`, so close tags are split defensively.
    let script = format!(
        r#"(function(){{
var frame=document.getElementById('view');
frame.srcdoc={srcdoc};
var result={result};
function post(m){{frame.contentWindow.postMessage(m,'*');}}
window.addEventListener('message',function(e){{
 if(e.source!==frame.contentWindow)return;var d=e.data||{{}};
 if(d.jsonrpc!=='2.0'||!d.method)return;
 if(d.method==='ui/initialize'&&d.id!==undefined){{post({{jsonrpc:'2.0',id:d.id,result:{{protocolVersion:'2026-01-26',hostInfo:{{name:'Everruns',version:'1'}},hostCapabilities:{{}},hostContext:{{theme:'light',displayMode:'inline',availableDisplayModes:['inline'],containerDimensions:{{width:{width},maxHeight:{height}}},platform:'web',toolInfo:{{tool:{{name:{tool},inputSchema:{{type:'object'}}}}}}}}}}}});return;}}
 if(d.method==='ui/notifications/initialized'){{post({{jsonrpc:'2.0',method:'ui/notifications/tool-input',params:{{arguments:{{}}}}}});post({{jsonrpc:'2.0',method:'ui/notifications/tool-result',params:result}});return;}}
 if(d.method==='ui/notifications/size-changed'&&typeof d.params==='object'&&d.params&&typeof d.params.height==='number'){{frame.style.height=Math.min({height},Math.max(120,Math.ceil(d.params.height)))+'px';return;}}
 if(d.id!==undefined)post({{jsonrpc:'2.0',id:d.id,error:{{code:-32601,message:'not available'}}}});
}});
}})();"#
    )
    .replace("</", "<\\/");
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><style>html,body{{margin:0;background:#fff}}#view{{display:block;width:{width}px;height:{height}px;border:0}}</style></head><body><iframe id="view" sandbox="allow-scripts" referrerpolicy="no-referrer"></iframe><script>{script}</script></body></html>"#
    )
}

fn view_csp(policy: &McpAppViewPolicyDto) -> String {
    let list = |origins: &[String]| origins.join(" ");
    let resources = list(&policy.resource_domains);
    let connect = list(&policy.connect_domains);
    let frames = list(&policy.frame_domains);
    let or_none = |value: String| {
        if value.is_empty() {
            "'none'".to_string()
        } else {
            value
        }
    };
    [
        "default-src 'none'".to_string(),
        format!("script-src 'unsafe-inline' 'unsafe-eval' blob: {resources}"),
        format!("style-src 'unsafe-inline' {resources}"),
        format!("img-src data: blob: {resources}"),
        format!("font-src data: {resources}"),
        format!("media-src data: blob: {resources}"),
        format!("connect-src {}", or_none(connect)),
        format!("frame-src {}", or_none(frames)),
        "base-uri 'none'".to_string(),
        "form-action 'none'".to_string(),
    ]
    .iter()
    .map(|directive| directive.trim().to_string())
    .collect::<Vec<_>>()
    .join("; ")
}

fn inject_csp(html: &str, csp: &str) -> String {
    let meta = format!(
        r#"<meta http-equiv="Content-Security-Policy" content="{}">"#,
        csp.replace('"', "&quot;")
    );
    let lower = html.to_ascii_lowercase();
    if let Some(start) = lower.find("<head")
        && let Some(end) = lower[start..].find('>')
    {
        let at = start + end + 1;
        return format!("{}{meta}{}", &html[..at], &html[at..]);
    }
    format!("<!DOCTYPE html><html><head>{meta}</head><body>{html}</body></html>")
}

/// Pictures of views for channels without a browser. A trait so channel
/// tests can stand in for the renderer.
#[async_trait::async_trait]
pub trait McpAppSnapshots: Send + Sync {
    async fn snapshot(
        &self,
        org_id: i64,
        session_id: SessionId,
        app: &mcp_apps::McpAppCallDetails,
        text: &str,
    ) -> McpAppSnapshot;
}

#[async_trait::async_trait]
impl McpAppSnapshots for SessionCommandService {
    async fn snapshot(
        &self,
        org_id: i64,
        session_id: SessionId,
        app: &mcp_apps::McpAppCallDetails,
        text: &str,
    ) -> McpAppSnapshot {
        self.snapshot_mcp_app_view(org_id, session_id, app, text)
            .await
    }
}

impl SessionCommandService {
    /// A picture of a view for a channel without a browser (Slack). Renders
    /// through the session owner's Browserless connection, the same one the
    /// agent's browser tools use; without one the channel posts the link only.
    pub async fn snapshot_mcp_app_view(
        &self,
        org_id: i64,
        session_id: SessionId,
        app: &mcp_apps::McpAppCallDetails,
        text: &str,
    ) -> McpAppSnapshot {
        if !self.interactive_views(org_id, &app.server).await.in_slack() {
            return McpAppSnapshot::Off;
        }
        let caller = Caller::internal(org_id);
        let view = match self
            .read_mcp_app_view(&caller, session_id, &app.server, &app.resource_uri)
            .await
        {
            Ok(view) => view,
            Err(error) => {
                tracing::info!(%session_id, %error, "MCP App view could not be read for a picture");
                return McpAppSnapshot::LinkOnly;
            }
        };
        let resolver = self.adapters().connection_resolver(org_id);
        let token = match resolver
            .get_connection_token(session_id, "browserless")
            .await
        {
            Ok(Some(token)) => token,
            Ok(None) => return McpAppSnapshot::LinkOnly,
            Err(error) => {
                tracing::info!(%session_id, %error, "no Browserless connection for an MCP App picture");
                return McpAppSnapshot::LinkOnly;
            }
        };
        let result = call_tool_result(&everruns_contracts::ToolResult {
            result: Some(json!({"result": text})),
            details: Some(json!({mcp_apps::MCP_RESULT_DETAILS_KEY: {
                "structured_content": app.structured_content,
                "meta": app.meta,
            }})),
            ..Default::default()
        });
        let page = snapshot_page(&view, &app.tool, &result);
        let client = everruns_integrations::browserless::client::BrowserlessClient::new(token);
        match client
            .screenshot_html(&page, "#view", SNAPSHOT_VIEWPORT, SNAPSHOT_SETTLE_MS)
            .await
        {
            Ok(png) if !png.is_empty() => McpAppSnapshot::Picture(png),
            Ok(_) => McpAppSnapshot::LinkOnly,
            Err(error) => {
                tracing::warn!(%session_id, %error, "Browserless could not draw an MCP App view");
                McpAppSnapshot::LinkOnly
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(meta: Option<Value>, read_only: Option<bool>) -> McpToolDefinition {
        serde_json::from_value(json!({
            "name": "t",
            "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": read_only},
            "_meta": meta,
        }))
        .unwrap()
    }

    #[test]
    fn read_only_and_app_only_tools_run_without_asking() {
        assert_eq!(
            app_call_policy(&tool(None, Some(true))),
            AppCallPolicy::Direct
        );
        assert_eq!(
            app_call_policy(&tool(Some(json!({"ui": {"visibility": ["app"]}})), None)),
            AppCallPolicy::Direct
        );
    }

    #[test]
    fn other_tools_need_confirmation_and_model_only_tools_are_refused() {
        assert_eq!(app_call_policy(&tool(None, None)), AppCallPolicy::Confirm);
        assert_eq!(
            app_call_policy(&tool(None, Some(false))),
            AppCallPolicy::Confirm
        );
        assert_eq!(
            app_call_policy(&tool(
                Some(json!({"ui": {"visibility": ["model"]}})),
                Some(true)
            )),
            AppCallPolicy::Forbidden
        );
    }

    #[test]
    fn call_tool_result_carries_text_structured_content_and_errors() {
        let result = everruns_contracts::ToolResult {
            tool_call_id: "c".into(),
            result: Some(json!({"result": "3 events"})),
            details: Some(
                json!({"mcp_result": {"structured_content": {"n": 3}, "meta": {"k": 1}}}),
            ),
            ..Default::default()
        };
        let out = call_tool_result(&result);
        assert_eq!(out["content"][0]["text"], "3 events");
        assert_eq!(out["structuredContent"], json!({"n": 3}));
        assert_eq!(out["_meta"], json!({"k": 1}));
        assert_eq!(out["isError"], false);

        let failed = everruns_contracts::ToolResult {
            tool_call_id: "c".into(),
            result: Some(json!({"error": "boom"})),
            ..Default::default()
        };
        let out = call_tool_result(&failed);
        assert_eq!(out["isError"], true);
        assert_eq!(out["content"][0]["text"], "boom");
    }

    #[test]
    fn snapshot_page_sandboxes_the_view_and_keeps_its_csp() {
        let view = McpAppView {
            html: "<html><head><title>x</title></head><body><script>1</script></body></html>"
                .into(),
            policy: McpAppViewPolicyDto {
                connect_domains: vec!["https://sentry.io".into()],
                ..Default::default()
            },
            interactive_views: McpInteractiveViews::On,
            can_call_tools: false,
        };
        let page = snapshot_page(&view, "render_chart", &json!({"content": []}));
        assert!(page.contains(r#"sandbox="allow-scripts""#));
        assert!(!page.contains("allow-same-origin"));
        assert!(page.contains("connect-src https://sentry.io"));
        assert!(page.contains("frame-src 'none'"));
        // The view's own closing tags never end the host's script early.
        let script = page.split("<script>").nth(1).unwrap();
        assert!(!script.split("</script>").next().unwrap().contains("</"));
    }
}
