mod bridge;
pub(super) mod workspace;
// MCP resources host the shared product UI through a source-checked bridge.
// The frontend receives no credentials: app_read/app_write enter the existing
// REST handlers with the authenticated MCP caller and resolved organization.
// Host capabilities, not client names, govern links and display modes.
// See knowledge/ui/mcp-apps.md and THREAT[TM-MCP-004].

use super::{AppState, ResolvedOrg, dispatch_command, form_elicitation, link_builder, mcp_ctx};
use crate::channels::slack::approvals::{ApprovalDecision, ApprovalPrompt, approval_prompt};
use crate::domains::common::Command;
use crate::domains::sessions::record::SessionStatus;
use everruns_contracts::typed_id::SessionId;
use everruns_core::Caller;
use everruns_core::builtins::ask_user::{AskUserAnswer, AskUserQuestionKind, AskUserStatus};
use serde_json::{Value, json};
use std::sync::OnceLock;

/// MCP Apps extension identifier (SEP-1865).
pub(super) const UI_EXTENSION_KEY: &str = "io.modelcontextprotocol/ui";
/// The only MIME type SEP-1865 defines for app templates.
pub(super) const APP_MIME_TYPE: &str = "text/html;profile=mcp-app";

/// Template for a single session: status, transcript tail, question, approval.
pub(super) const SESSION_VIEW_URI: &str = "ui://everruns/app/session";
/// Template for one agent card.
pub(super) const AGENT_VIEW_URI: &str = "ui://everruns/app/agent";
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
    if let Some(uri) = match name {
        "harness_get_card" => Some(workspace::HARNESS_URI),
        "capability_get_card" => Some(workspace::CAPABILITY_URI),
        "knowledge_get_card" => Some(workspace::KNOWLEDGE_URI),
        "session_open" => Some(SESSION_VIEW_URI),
        _ => None,
    } {
        return Some(json!({"ui":{"resourceUri":uri}}));
    }
    if matches!(name, "app_read" | "app_write" | "app_bootstrap") {
        return Some(json!({"ui":{"visibility":["app"]}}));
    }
    if name == "agent_get_card" {
        return Some(json!({ "ui": { "resourceUri": AGENT_VIEW_URI } }));
    }
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

/// Presentation hints. Resources/read adds the deployment-specific frame CSP.
fn resource_meta(uri: &str) -> Value {
    let preferred = if matches!(uri, HOME_VIEW_URI | SESSION_VIEW_URI) {
        "fullscreen"
    } else {
        "inline"
    };
    json!({
        "ui": {
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
            workspace::HARNESS_URI,
            "Everruns harness",
            "A harness card.",
        ),
        (
            workspace::CAPABILITY_URI,
            "Everruns capability",
            "A capability card.",
        ),
        (
            workspace::KNOWLEDGE_URI,
            "Everruns knowledge",
            "A knowledge collection card.",
        ),
        (
            AGENT_VIEW_URI,
            "Everruns agent",
            "Agent identity, channels, and invocation activity.",
        ),
        (
            SESSION_VIEW_URI,
            "Everruns session",
            "Full Session UI, including Trace, Approvals, Work, Events, Files and Cost.",
        ),
        (HOME_VIEW_URI, "Everruns home", "Full organization Home UI."),
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

/// Run one of this module's tools and return its structured result.
pub(super) async fn call_tool(
    name: &str,
    args: &Value,
    org: &ResolvedOrg,
    state: &AppState,
) -> Result<Value, String> {
    match name {
        HOME_TOOL => Ok(json!({"view":{"kind":"home"},"organization_id":org.public_id})),
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
                "conversation.message".to_string(),
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
        "organization_id": org.public_id,
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

fn transcript_tail(events: &[crate::storage::EventRow]) -> Vec<Value> {
    let mut messages: Vec<Value> = events
        .iter()
        .filter_map(|event| {
            // What the agent said, never its commentary: its text in direct
            // communication, its sent messages in explicit communication.
            let (role, text) = match event.event_type.as_str() {
                "input.message" => ("user", message_text(&event.data)?),
                other => (
                    "agent",
                    everruns_core::conversation::said_in_event(other, &event.data)?,
                ),
            };
            Some(json!({ "role": role, "text": text }))
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
) -> Result<Option<ApprovalPrompt>, String> {
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

fn latest_unanswered_approval(events: &[crate::storage::EventRow]) -> Option<ApprovalPrompt> {
    let mut pending = None;
    for event in events {
        match event.event_type.as_str() {
            "input.message" => pending = None,
            "tool.completed" => {
                if let Some(request) = approval_prompt(&event.data) {
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
            "organization_id": org.public_id,
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
    use crate::storage::EventRow;

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
    fn agent_card_has_a_readable_app_template() {
        let meta = tool_meta("agent_get_card").expect("agent cards must open an MCP App");
        let uri = meta["ui"]["resourceUri"].as_str().unwrap();
        let read = workspace::resource(uri, "https://app.everruns.com")
            .expect("the advertised template must be readable");
        assert_eq!(read["contents"][0]["mimeType"], APP_MIME_TYPE);
        assert!(
            resource_list_entries()
                .iter()
                .any(|entry| entry["uri"] == uri)
        );
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
    fn templates_embed_only_the_configured_product_origin() {
        for uri in [SESSION_VIEW_URI, HOME_VIEW_URI, AGENT_VIEW_URI] {
            let read = workspace::resource(uri, "https://app.everruns.com").unwrap();
            let content = &read["contents"][0];
            assert_eq!(content["mimeType"], APP_MIME_TYPE);
            assert_eq!(content["_meta"]["ui"]["csp"]["connectDomains"], json!([]));
            let html = content["text"].as_str().unwrap();
            assert_eq!(
                content["_meta"]["ui"]["csp"]["frameDomains"],
                json!(["https://app.everruns.com"])
            );
            assert!(html.contains("https://app.everruns.com/mcp-app"));
            assert!(html.len() < super::super::cards::MAX_CARD_BYTES);
        }
        assert!(
            workspace::resource(
                "ui://everruns/agent/agent_1/card",
                "https://app.everruns.com"
            )
            .is_none()
        );
        assert_eq!(resource_list_entries().len(), 6);
    }

    #[test]
    fn template_never_writes_server_data_as_html() {
        // The view renders agent- and user-authored text. `innerHTML` would
        // make every message an XSS vector inside the host's sandbox.
        assert!(!include_str!("workspace.html").contains("innerHTML"));
        assert!(!include_str!("workspace.html").contains("outerHTML"));
        assert!(!include_str!("workspace.html").contains("insertAdjacentHTML"));
        assert!(!include_str!("workspace.html").contains("document.write"));
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
}
