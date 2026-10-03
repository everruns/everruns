// MCP Apps views for Everruns' own MCP server (EVE-1122).
//
// See `knowledge/ui/mcp-apps.md`. Decisions:
//
// - **One implementation, the open standard.** ChatGPT, Codex and Claude all
//   render MCP Apps (SEP-1865): a tool names a `ui://` template in
//   `_meta.ui.resourceUri`, the host reads it with `resources/read`
//   (`text/html;profile=mcp-app`) and feeds it the tool result over a
//   postMessage JSON-RPC bridge. OpenAI's extensions ride on top as extra
//   `_meta["openai/ui"]` keys that other hosts ignore, so nothing here branches
//   on which host is calling. The view itself branches on the capabilities the
//   host hands it in `ui/initialize` (can it proxy tool calls, open links, go
//   fullscreen), never on a user agent.
// - **Static templates, dynamic data.** The HTML is the same for every caller
//   and carries no data, so it is safe to prefetch and cache. Data arrives as
//   `structuredContent` from ordinary tool results.
// - **The view is never the trust boundary.** Buttons ask the host to call an
//   app-only tool on this server; the call arrives through `/mcp` with the
//   user's own MCP credential and passes the same org resolution and command
//   policy as any other call (THREAT[TM-MCP-004]). There is no side channel,
//   no cookie, and nothing a cross-site request could ride.
// - **Answers go through the existing pause-and-resume paths.** A question is
//   resolved by the one shared `resolve_question_answers` operation, and an
//   approval becomes the user's next message, exactly like a Slack click
//   (`slack_approvals`). The view adds a surface, not a protocol.

use super::{
    AppState, ResolvedOrg, dispatch_command, form_elicitation, link_builder, mcp_ctx,
    resource_error,
};
use crate::domains::common::Command;
use crate::slack_approvals::{ApprovalDecision, ApprovalRequest, extract_approval_request};
use everruns_builtins::ask_user::{AskUserAnswer, AskUserQuestionKind, AskUserStatus};
use everruns_contracts::typed_id::SessionId;
use everruns_core::Caller;
use everruns_platform::SessionStatus;
use serde_json::{Value, json};
use std::sync::OnceLock;

/// MCP Apps extension identifier (SEP-1865).
pub(super) const UI_EXTENSION_KEY: &str = "io.modelcontextprotocol/ui";
/// The only MIME type SEP-1865 defines for app templates.
pub(super) const APP_MIME_TYPE: &str = "text/html;profile=mcp-app";

/// Template for a single session: status, transcript tail, question, approval.
pub(super) const SESSION_VIEW_URI: &str = "ui://everruns/app/session";
/// Template for the home panel: agents, recent sessions, what waits on you.
pub(super) const HOME_VIEW_URI: &str = "ui://everruns/app/home";

/// Model-visible entrypoint that opens the home panel.
pub(super) const HOME_TOOL: &str = "everruns_home";
/// App-only: refresh one session's view state.
pub(super) const SESSION_VIEW_TOOL: &str = "session_view";
/// App-only: answer a parked `ask_user` question set.
pub(super) const ANSWER_TOOL: &str = "session_answer_question";
/// App-only: approve or decline a `request_approval` pause.
pub(super) const APPROVAL_TOOL: &str = "session_decide_approval";

/// Tools whose results the session view renders.
const SESSION_VIEW_TOOLS: &[&str] = &[
    "agent_run",
    "session_send_message",
    "session_get_status",
    SESSION_VIEW_TOOL,
    ANSWER_TOOL,
    APPROVAL_TOOL,
];

/// Transcript messages carried in a session view. The view is a glance, not
/// the full history: "Open in Everruns" is there for the rest.
const VIEW_MESSAGE_LIMIT: usize = 12;
/// Events read to build a view. Covers the transcript tail and a pending
/// approval raised in the last turn.
const VIEW_EVENT_LOOKBACK: i32 = 80;
/// Sessions listed on the home panel.
const HOME_SESSION_LIMIT: u32 = 10;
/// Agents listed on the home panel.
const HOME_AGENT_LIMIT: u32 = 25;

const APP_HTML: &str = include_str!("apps/app.html");

/// 20x20 monochrome sidebar icon, `currentColor` so it follows the host theme
/// (OpenAI extensions spec, "Icon Guidelines").
const ICON_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.33" stroke-linecap="round" stroke-linejoin="round"><circle cx="10" cy="10" r="7.33"/><path d="M7 10h6M10 7l3 3-3 3"/></svg>"#;

fn icon_data_uri() -> &'static str {
    static URI: OnceLock<String> = OnceLock::new();
    URI.get_or_init(|| {
        use base64::Engine as _;
        format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(ICON_SVG)
        )
    })
}

/// Whether this request's per-request capabilities declare MCP Apps support.
///
/// Only the 2026-07-28 protocol carries capabilities on every request; an
/// older client declared them once at `initialize`, which a stateless server
/// never sees again. Nothing here depends on the answer for correctness:
/// hosts without MCP Apps ignore `_meta.ui` and read the text content.
pub(super) fn client_supports_apps(params: &Value) -> bool {
    params
        .get("_meta")
        .and_then(|meta| meta.get("io.modelcontextprotocol/clientCapabilities"))
        .and_then(|caps| caps.get("extensions"))
        .and_then(|extensions| extensions.get(UI_EXTENSION_KEY))
        .is_some()
}

/// Whether `name` is one of the tools this module serves.
pub(super) fn is_app_tool(name: &str) -> bool {
    matches!(
        name,
        HOME_TOOL | SESSION_VIEW_TOOL | ANSWER_TOOL | APPROVAL_TOOL
    )
}

/// `_meta` for a tool descriptor in `tools/list`, or `None` for tools with no
/// view.
pub(super) fn tool_meta(name: &str) -> Option<Value> {
    if name == HOME_TOOL {
        return Some(json!({
            "ui": { "resourceUri": HOME_VIEW_URI },
            // OpenAI MCP extensions: a sidebar entry and a thread tab. Both
            // open the tool with `{}`, which `everruns_home` accepts.
            "openai/ui": { "entrypoints": [{ "type": "global" }, { "type": "thread" }] },
        }));
    }
    if !SESSION_VIEW_TOOLS.contains(&name) {
        return None;
    }
    let mut ui = json!({ "resourceUri": SESSION_VIEW_URI });
    if matches!(name, SESSION_VIEW_TOOL | ANSWER_TOOL | APPROVAL_TOOL) {
        // Buttons in the view, not something the model should call: the model
        // has `session_get_status` and `session_send_message` for that.
        ui["visibility"] = json!(["app"]);
    }
    Some(json!({ "ui": ui }))
}

/// Tool `icons` (MCP `Icon[]`) for entrypoint tools.
pub(super) fn tool_icons(name: &str) -> Option<Value> {
    (name == HOME_TOOL).then(|| {
        json!([{
            "src": icon_data_uri(),
            "mimeType": "image/svg+xml",
            "sizes": ["any"],
        }])
    })
}

/// The server extensions map entry for MCP Apps.
pub(super) fn server_extension() -> Value {
    json!({ "mimeTypes": [APP_MIME_TYPE] })
}

/// Security and rendering metadata for a template.
///
/// The view needs no network at all: it reaches the server only through the
/// host. An empty CSP declaration therefore asks for the spec's restrictive
/// default, and the document repeats it in a `<meta>` tag for hosts that only
/// sandbox.
fn resource_meta(uri: &str) -> Value {
    let preferred = if uri == HOME_VIEW_URI {
        "fullscreen"
    } else {
        "inline"
    };
    json!({
        "ui": {
            "csp": { "connectDomains": [], "resourceDomains": [] },
            "prefersBorder": true,
        },
        "openai/ui": {
            "availableDisplayModes": ["inline", "fullscreen"],
            "preferredDisplayMode": preferred,
        },
    })
}

/// Entries for `resources/list`.
pub(super) fn resource_list_entries() -> Vec<Value> {
    [
        (
            SESSION_VIEW_URI,
            "Everruns session",
            "Live view of one session: status, recent messages, and any question or approval waiting on the user.",
        ),
        (
            HOME_VIEW_URI,
            "Everruns home",
            "Agents, recent sessions, and everything waiting on the user.",
        ),
    ]
    .into_iter()
    .map(|(uri, name, description)| {
        json!({
            "uri": uri,
            "name": name,
            "description": description,
            "mimeType": APP_MIME_TYPE,
            "_meta": resource_meta(uri),
        })
    })
    .collect()
}

/// `resources/read` contents for a template URI, or `None` for other URIs.
pub(super) fn read_resource(uri: &str) -> Option<Value> {
    if uri != SESSION_VIEW_URI && uri != HOME_VIEW_URI {
        return None;
    }
    Some(json!({
        "contents": [{
            "uri": uri,
            "mimeType": APP_MIME_TYPE,
            "text": APP_HTML,
            "_meta": resource_meta(uri),
        }]
    }))
}

/// Run one of this module's tools and return its structured result.
pub(super) async fn call_tool(
    name: &str,
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    match name {
        HOME_TOOL => home(org, state).await,
        SESSION_VIEW_TOOL => session_view(&session_id_arg(args)?, org, state).await,
        ANSWER_TOOL => answer_question(args, org, state).await,
        APPROVAL_TOOL => decide_approval(args, org, state).await,
        _ => Err(format!("Unknown tool: {name}")),
    }
}

fn session_id_arg(args: &Value) -> Result<SessionId, String> {
    let raw = args
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or("Missing required parameter: session_id")?;
    raw.parse()
        .map_err(|_| format!("Invalid session_id: {raw}"))
}

fn policy_error(error: everruns_core::PolicyError) -> String {
    error.to_string()
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

async fn home(org: &ResolvedOrg, state: &AppState) -> Result<Value, String> {
    let ctx = mcp_ctx(org, state);
    let agents = crate::domains::agents::ListAgents {
        search: None,
        include_archived: false,
        offset: Some(0),
        limit: Some(HOME_AGENT_LIMIT),
    }
    .run(&ctx)
    .await
    .map_err(|e| resource_error("agents", e))?;

    let agent_names: std::collections::HashMap<String, String> = agents
        .data
        .iter()
        .map(|agent| {
            (
                agent.public_id.to_string(),
                agent
                    .display_name
                    .clone()
                    .unwrap_or_else(|| agent.name.clone()),
            )
        })
        .collect();

    let sessions = dispatch_command(
        "list_sessions",
        json!({ "mine": true, "order": "last_activity", "limit": HOME_SESSION_LIMIT }),
        org,
        state,
    )
    .await?;

    let caller = Caller::from(org);
    let mut session_items = Vec::new();
    for session in sessions["data"].as_array().into_iter().flatten() {
        let Some(raw_id) = session["id"].as_str() else {
            continue;
        };
        let Ok(session_id) = raw_id.parse::<SessionId>() else {
            continue;
        };
        let status = normalize_status(session["status"].as_str().unwrap_or_default());
        // Only a turn that stopped can be waiting on the user, so running
        // sessions skip the event read.
        let pending = if matches!(status.as_str(), "idle" | "waiting_for_tool_results") {
            pending_input(&caller, session_id, &status, state)
                .await
                .unwrap_or(None)
        } else {
            None
        };
        let agent_id = session["agent_id"].as_str().map(str::to_string);
        session_items.push(json!({
            "session_id": raw_id,
            "title": session["title"],
            "status": status,
            "agent_id": agent_id,
            "agent_name": agent_id.as_ref().and_then(|id| agent_names.get(id)),
            "pending": pending.map(|pending| json!({ "kind": pending["kind"] })),
        }));
    }

    let agent_items: Vec<Value> = agents
        .data
        .iter()
        .map(|agent| {
            json!({
                "id": agent.public_id.to_string(),
                "name": agent.name,
                "display_name": agent.display_name,
                "description": agent.description,
            })
        })
        .collect();

    Ok(json!({
        "organization": { "id": org.public_id, "name": org.name },
        "agents": agent_items,
        "sessions": session_items,
    }))
}

/// Session status as the view vocabulary spells it. The JSON enum and the
/// `Display` form disagree on the waiting state; the view only knows one.
fn normalize_status(status: &str) -> String {
    match status {
        "waitingfortoolresults" => "waiting_for_tool_results".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Session view
// ---------------------------------------------------------------------------

/// The state the session view renders.
pub(super) async fn session_view(
    session_id: &SessionId,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    let caller = Caller::from(org);
    crate::domains::sessions::SESSION_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(policy_error)?;
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let status = session.status.to_string();

    let events = state
        .db
        .list_events(
            *session_id,
            None,
            None,
            &[
                "input.message".to_string(),
                "output.message.completed".to_string(),
                "tool.completed".to_string(),
            ],
            &[],
            None,
            Some(VIEW_EVENT_LOOKBACK),
        )
        .await
        .map_err(|error| error.to_string())?;

    let messages = transcript_tail(&events);
    let pending = pending_input(&caller, *session_id, &status, state).await?;

    let agent_name = match session.agent_id {
        Some(agent_id) => {
            let ctx = mcp_ctx(org, state);
            crate::domains::agents::GetAgent {
                id: agent_id.to_string(),
            }
            .run(&ctx)
            .await
            .ok()
            .map(|agent| agent.display_name.unwrap_or(agent.name))
        }
        None => None,
    };

    let mut view = json!({
        "session_id": session_id.to_string(),
        "title": session.title,
        "status": status,
        "agent_id": session.agent_id.map(|id| id.to_string()),
        "agent_name": agent_name,
        "messages": messages,
        "pending": pending,
    });
    link_builder(state).decorate_value_links(&mut view);
    Ok(view)
}

/// Text of a message event's content parts.
fn message_text(event_data: &Value) -> Option<String> {
    let parts = event_data.get("message")?.get("content")?.as_array()?;
    let text = parts
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn transcript_tail(events: &[crate::storage::models::EventRow]) -> Vec<Value> {
    let mut messages: Vec<Value> = events
        .iter()
        .filter_map(|event| {
            let role = match event.event_type.as_str() {
                "input.message" => "user",
                "output.message.completed" => "agent",
                _ => return None,
            };
            Some(json!({ "role": role, "text": message_text(&event.data)? }))
        })
        .collect();
    let skip = messages.len().saturating_sub(VIEW_MESSAGE_LIMIT);
    messages.drain(..skip);
    messages
}

/// What the session is waiting on the user for, if anything.
async fn pending_input(
    caller: &Caller,
    session_id: SessionId,
    status: &str,
    state: &AppState,
) -> Result<Option<Value>, String> {
    if status == "waiting_for_tool_results" {
        let Some(pending) =
            form_elicitation::pending_questions_for_session(caller, session_id, state).await?
        else {
            return Ok(None);
        };
        // THREAT[TM-AGENT-016]: a credential is never typed into a view. The
        // secret question is left to the Everruns page, which stores the value
        // without it ever reaching the host, the model, or the event log.
        if pending
            .questions
            .iter()
            .any(|question| question.kind == AskUserQuestionKind::Secret)
        {
            return Ok(Some(json!({
                "kind": "unsupported",
                "message": "The agent needs a secret. Open the session in Everruns to provide it.",
            })));
        }
        let questions: Vec<Value> = pending
            .questions
            .iter()
            .map(|question| {
                json!({
                    "id": question.id,
                    "kind": match question.kind {
                        AskUserQuestionKind::Text => "text",
                        _ => "choice",
                    },
                    "header": question.header,
                    "question": question.question,
                    "multi_select": question.multi_select,
                    "allow_other": question.allow_other,
                    "options": question.options.iter().map(|option| json!({
                        "label": option.label,
                        "description": option.description,
                        "is_default": option.is_default,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        return Ok(Some(json!({
            "kind": "question",
            "tool_call_id": pending.tool_call_id,
            "questions": questions,
        })));
    }
    if status == "idle"
        && let Some(request) = pending_approval(session_id, state).await?
    {
        return Ok(Some(json!({
            "kind": "approval",
            "action": request.action,
            "question": request.question,
        })));
    }
    Ok(None)
}

/// A `request_approval` pause nobody has answered yet.
///
/// The turn that raised it is over, so "unanswered" means no user message
/// arrived after it. The same read backs both rendering the buttons and
/// checking a click against what is still pending.
async fn pending_approval(
    session_id: SessionId,
    state: &AppState,
) -> Result<Option<ApprovalRequest>, String> {
    let events = state
        .db
        .list_events(
            session_id,
            None,
            None,
            &["input.message".to_string(), "tool.completed".to_string()],
            &[],
            None,
            Some(VIEW_EVENT_LOOKBACK),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(latest_unanswered_approval(&events))
}

fn latest_unanswered_approval(
    events: &[crate::storage::models::EventRow],
) -> Option<ApprovalRequest> {
    let mut pending = None;
    for event in events {
        match event.event_type.as_str() {
            "input.message" => pending = None,
            "tool.completed" => {
                if let Some(request) = extract_approval_request(&event.data) {
                    pending = Some(request);
                }
            }
            _ => {}
        }
    }
    pending
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

async fn answer_question(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    use crate::api::question_answers::{QuestionResolver, ResolveError, resolve_question_answers};

    let session_id = session_id_arg(args)?;
    let caller = Caller::from(org);
    crate::domains::sessions::SESSION_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(policy_error)?;

    let status = match args
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("answered")
    {
        "answered" => AskUserStatus::Answered,
        "declined" => AskUserStatus::Declined,
        other => {
            return Err(format!(
                "Invalid status {other:?}: use answered or declined"
            ));
        }
    };
    let tool_call_id = args.get("tool_call_id").and_then(Value::as_str);
    let answers: Vec<AskUserAnswer> = if status == AskUserStatus::Answered {
        args.get("answers")
            .and_then(Value::as_array)
            .ok_or("Missing required parameter: answers")?
            .iter()
            .map(|answer| {
                Ok(AskUserAnswer {
                    id: answer
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("Every answer needs an id")?
                        .to_string(),
                    selected: answer
                        .get("selected")
                        .and_then(Value::as_array)
                        .map(|labels| {
                            labels
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    other_text: answer
                        .get("other_text")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    // THREAT[TM-AGENT-016]: a view never answers a secret
                    // question, so it never carries a secret reference either.
                    secret_ref: None,
                })
            })
            .collect::<Result<_, String>>()?
    } else {
        Vec::new()
    };

    let resolver = QuestionResolver {
        db: &state.db,
        session_service: &state.session_service,
        event_service: &state.event_service,
        runner: state.runner.clone(),
    };
    // Validation against what was actually asked, attribution, the single-claim
    // resume: all owned by the shared operation (THREAT[TM-AGENT-015]).
    resolve_question_answers(
        &resolver,
        &caller,
        session_id,
        tool_call_id,
        status,
        &answers,
    )
    .await
    .map_err(|error| match error {
        ResolveError::Invalid(detail) => detail,
        ResolveError::AlreadyResolved => "This question was already answered.".to_string(),
        ResolveError::NoPendingQuestions | ResolveError::WrongPendingCall => {
            "The agent is no longer waiting on this question.".to_string()
        }
        ResolveError::NotWaiting(_) => "The session is not waiting on a question.".to_string(),
        ResolveError::Internal(detail) => {
            tracing::error!(error = %detail, "Failed to resolve an MCP app answer");
            "Failed to record the answer".to_string()
        }
    })?;

    session_view(&session_id, org, state).await
}

async fn decide_approval(
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    let session_id = session_id_arg(args)?;
    let caller = Caller::from(org);
    crate::domains::sessions::SESSION_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(policy_error)?;

    let decision = match args.get("decision").and_then(Value::as_str) {
        Some("approve") => ApprovalDecision::Approved,
        Some("decline") => ApprovalDecision::Declined,
        _ => return Err("decision must be \"approve\" or \"decline\"".to_string()),
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or("Missing required parameter: action")?;

    // Reading the session through the caller also enforces tenancy before we
    // touch its events.
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    if session.status != SessionStatus::Idle {
        return Err("The session is not waiting on an approval.".to_string());
    }
    // THREAT[TM-MCP-004]: a click answers only the pause that was on screen.
    // The view echoes the action it showed; if the pending one differs (a
    // newer turn asked something else, or someone already answered), the click
    // is stale and must not approve whatever is pending now.
    let pending = pending_approval(session_id, state)
        .await?
        .ok_or("The agent is no longer waiting on this approval.")?;
    if pending.action != action.trim() {
        return Err(
            "This approval is out of date. Refresh to see what the agent is asking now."
                .to_string(),
        );
    }

    // The decision is an ordinary user message from the authenticated caller,
    // so `approval_audit` attributes it the same way it does a typed "yes".
    dispatch_command(
        "create_message",
        json!({
            "session_id": session_id.to_string(),
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": decision.as_message(&pending.action) }]
            }
        }),
        org,
        state,
    )
    .await?;

    session_view(&session_id, org, state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::EventRow;

    fn event(sequence: i32, event_type: &str, data: Value) -> EventRow {
        EventRow {
            id: everruns_contracts::typed_id::EventId::from_uuid(uuid::Uuid::now_v7()),
            session_id: SessionId::from_uuid(uuid::Uuid::nil()),
            sequence,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn user_message(text: &str) -> Value {
        json!({ "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
    }

    fn approval_completed(action: &str) -> Value {
        let payload = json!({ "awaiting_approval": true, "action": action, "question": "OK?" });
        json!({
            "tool_name": "request_approval",
            "success": true,
            "result": [{ "type": "text", "text": payload.to_string() }]
        })
    }

    #[test]
    fn session_tools_point_at_the_session_view() {
        for tool in ["agent_run", "session_send_message", "session_get_status"] {
            let meta = tool_meta(tool).expect("session tools carry a view");
            assert_eq!(meta["ui"]["resourceUri"], SESSION_VIEW_URI);
            // Model-visible: no visibility restriction.
            assert!(meta["ui"].get("visibility").is_none());
        }
        assert!(tool_meta("discover").is_none());
    }

    #[test]
    fn action_tools_are_app_only() {
        for tool in [SESSION_VIEW_TOOL, ANSWER_TOOL, APPROVAL_TOOL] {
            let meta = tool_meta(tool).expect("view tool");
            assert_eq!(meta["ui"]["visibility"], json!(["app"]));
        }
    }

    #[test]
    fn home_is_a_global_and_thread_entrypoint() {
        let meta = tool_meta(HOME_TOOL).unwrap();
        assert_eq!(meta["ui"]["resourceUri"], HOME_VIEW_URI);
        assert_eq!(
            meta["openai/ui"]["entrypoints"],
            json!([{ "type": "global" }, { "type": "thread" }])
        );
        let icons = tool_icons(HOME_TOOL).unwrap();
        assert!(
            icons[0]["src"]
                .as_str()
                .unwrap()
                .starts_with("data:image/svg+xml;base64,")
        );
    }

    #[test]
    fn templates_are_mcp_app_resources_without_network_access() {
        for uri in [SESSION_VIEW_URI, HOME_VIEW_URI] {
            let read = read_resource(uri).unwrap();
            let content = &read["contents"][0];
            assert_eq!(content["mimeType"], APP_MIME_TYPE);
            assert_eq!(content["_meta"]["ui"]["csp"]["connectDomains"], json!([]));
            let html = content["text"].as_str().unwrap();
            assert!(html.contains("connect-src 'none'"));
            assert!(html.len() < super::super::cards::MAX_CARD_BYTES);
        }
        assert!(read_resource("ui://everruns/agent/agent_1/card").is_none());
        assert_eq!(resource_list_entries().len(), 2);
    }

    #[test]
    fn template_never_writes_server_data_as_html() {
        // The view renders agent- and user-authored text. `innerHTML` would
        // make every message an XSS vector inside the host's sandbox.
        assert!(!APP_HTML.contains("innerHTML"));
        assert!(!APP_HTML.contains("outerHTML"));
        assert!(!APP_HTML.contains("insertAdjacentHTML"));
        assert!(!APP_HTML.contains("document.write"));
    }

    #[test]
    fn apps_capability_is_read_from_request_meta() {
        let params = json!({ "_meta": { "io.modelcontextprotocol/clientCapabilities": {
            "extensions": { "io.modelcontextprotocol/ui": { "mimeTypes": [APP_MIME_TYPE] } }
        }}});
        assert!(client_supports_apps(&params));
        assert!(!client_supports_apps(&json!({})));
    }

    #[test]
    fn approval_is_pending_until_the_next_user_message() {
        let raised = vec![
            event(1, "input.message", user_message("deploy")),
            event(2, "tool.completed", approval_completed("Deploy to prod")),
        ];
        assert_eq!(
            latest_unanswered_approval(&raised).unwrap().action,
            "Deploy to prod"
        );

        let mut answered = raised.clone();
        answered.push(event(3, "input.message", user_message("Approved: Deploy")));
        assert!(latest_unanswered_approval(&answered).is_none());
    }

    #[test]
    fn transcript_tail_keeps_the_latest_messages_in_order() {
        let mut events = Vec::new();
        for i in 0..20 {
            events.push(event(i, "input.message", user_message(&format!("m{i}"))));
        }
        events.push(event(
            20,
            "tool.completed",
            json!({ "tool_name": "x", "success": true }),
        ));
        let tail = transcript_tail(&events);
        assert_eq!(tail.len(), VIEW_MESSAGE_LIMIT);
        assert_eq!(tail.last().unwrap()["text"], "m19");
        assert_eq!(tail.first().unwrap()["text"], "m8");
    }

    #[test]
    fn waiting_status_is_normalized() {
        assert_eq!(
            normalize_status("waitingfortoolresults"),
            "waiting_for_tool_results"
        );
        assert_eq!(normalize_status("idle"), "idle");
    }
}
