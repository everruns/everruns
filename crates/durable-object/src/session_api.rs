//! Transport-free Workers/session gateway. Every operation enters Framework
//! Engine/Session; this module maps protocol values and authors no agent events.
use everruns::hosting::spi::error::{AgentLoopError, Result};
use everruns::{
    Engine, InputMessage, MessageId, SessionEventCursor, SessionId, hosting::AgentBuild,
};
use serde_json::{Value, json};

#[derive(Debug)]
struct QueryBudgetExceeded;
impl std::fmt::Display for QueryBudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("event response exceeds byte budget; narrow the sequence range or limit")
    }
}
impl std::error::Error for QueryBudgetExceeded {}

/// Thin JSON adapter used by isolate and native HTTP hosting entrypoints.
pub struct SessionApi {
    engine: Engine,
    build: AgentBuild,
}

impl SessionApi {
    async fn summary(&self, session: SessionId) -> Result<Value> {
        let status = self.engine.session_status(session).await?;
        let phase = status.phase();
        let approvals:Vec<_>=status.waiting.iter().filter_map(|call|everruns::hosting::spi::approval::ToolApprovalRequired::from_request_call(&call.id,&call.name,&call.arguments).map(|request|json!({"tool_call_id":call.id,"tool_name":request.tool,"arguments":request.arguments}))).collect();
        let questions: Vec<_> = status
            .waiting
            .iter()
            .filter(|call| call.name == everruns::hosting::spi::ASK_USER_TOOL_NAME)
            .map(|call| json!({"tool_call_id":call.id,"questions":call.arguments["questions"]}))
            .collect();
        let metadata = status.binding.metadata;
        Ok(
            json!({"id":session,"organization_id":"org_00000000000000000000000000000000","agent_id":status.binding.behavior.agent,"agent_name":status.binding.behavior.agent,"build_id":status.binding.behavior.build_id,"status":phase,"turn_id":status.open_turn,"suspension_reason":status.suspension_reason,"created_at":status.created_at,"updated_at":status.created_at,"tags":metadata.get("tags").cloned().unwrap_or(json!([])),"title":metadata["title"],"hints":metadata["hints"],"metadata":metadata["metadata"],"pending_approvals":approvals,"pending_questions":questions,"pending_tools":status.waiting}),
        )
    }
    /// Protocol status with diagnostics kept on the host. Authority credentials
    /// are an internal concern, so their failure is not a client auth challenge.
    pub fn error_status(error: &AgentLoopError) -> u16 {
        if let AgentLoopError::Internal(error) = error {
            for cause in error.chain() {
                if cause.is::<QueryBudgetExceeded>() {
                    return 413;
                }
                if let Some(rejected) = cause.downcast_ref::<crate::client::AuthorityRejection>() {
                    return match rejected.0 {
                        404 | 409 | 413 | 429 => rejected.0,
                        _ => 503,
                    };
                }
                if let Some(error) = cause.downcast_ref::<AgentLoopError>() {
                    match error {
                        AgentLoopError::Configuration(_) => return 400,
                        AgentLoopError::SessionNotFound(_) => return 404,
                        _ => {}
                    }
                }
            }
        }
        match error {
            AgentLoopError::Configuration(_) => 400,
            AgentLoopError::SessionNotFound(_) => 404,
            _ => 503,
        }
    }
    pub fn new(engine: Engine, build: AgentBuild) -> Self {
        Self { engine, build }
    }

    pub async fn handle(&self, session: SessionId, operation: &str, body: Value) -> Result<Value> {
        if operation == "card" {
            if body["agent_name"]
                .as_str()
                .is_some_and(|name| name != self.build.agent)
            {
                return Err(crate::client::rejected(404));
            }
            use everruns::execution_api::{
                AgentCard, AgentCardAuth, AgentCardInput, AgentCardLinks,
            };
            return Ok(json!(AgentCard {
                name: self.build.agent.clone(),
                description: None,
                streaming: true,
                input: AgentCardInput::TEXT,
                auth: vec![AgentCardAuth::AgentKey],
                conversation_starters: Vec::new(),
                links: AgentCardLinks {
                    sessions: format!("/v1/channels/{}/sessions", self.build.agent),
                    ag_ui: None,
                    a2a: None
                }
            }));
        }
        if operation == "scope" {
            let binding = self.engine.session_binding(session).await?;
            if body["agent_name"].as_str() != Some(binding.behavior.agent.as_str()) {
                return Err(crate::client::rejected(404));
            }
            return Ok(json!({}));
        }
        if operation == "create" {
            if body
                .get("agent_name")
                .and_then(Value::as_str)
                .is_some_and(|name| name != self.build.agent)
            {
                return Err(AgentLoopError::config(
                    "agent is not registered at this endpoint",
                ));
            }
            self.engine
                .create_registered_with_metadata(
                    session,
                    &self.build,
                    body.get("creation").cloned().unwrap_or(Value::Null),
                )
                .await?;
            return self.summary(session).await;
        }
        if operation == "status" {
            return self.summary(session).await;
        }
        self.engine.session_binding(session).await?;
        let handle = self
            .engine
            .resume(session)
            .await
            .map_err(|_| AgentLoopError::store("session could not be reopened"))?;
        match operation {
            "send" => {
                let message: MessageId = serde_json::from_value(body["message_id"].clone())
                    .map_err(|_| AgentLoopError::config("stable message identity required"))?;
                let mut input = InputMessage::user("");
                input.content = serde_json::from_value(body["content"].clone())
                    .map_err(|_| AgentLoopError::config("invalid message content"))?;
                let content = serde_json::to_value(&input.content)
                    .map_err(|_| AgentLoopError::config("invalid message content"))?;
                let accepted = handle
                    .send_with_id(message, input)
                    .await
                    .map_err(|e| AgentLoopError::Internal(anyhow::Error::new(e)))?;
                let receipt = handle
                    .admission(message)
                    .await
                    .map_err(|e| AgentLoopError::Internal(anyhow::Error::new(e)))?
                    .ok_or_else(|| AgentLoopError::store("admission receipt unavailable"))?;
                Ok(
                    json!({"id":accepted.message_id,"session_id":session,"role":"user","content":content,"sequence":receipt.sequence()}),
                )
            }
            "admission" | "cancel" => {
                let message: MessageId = serde_json::from_value(body["message_id"].clone())
                    .map_err(|_| AgentLoopError::config("stable message identity required"))?;
                let admission = handle
                    .admission(message)
                    .await
                    .map_err(|e| AgentLoopError::Internal(anyhow::Error::new(e)))?
                    .ok_or_else(|| AgentLoopError::config("admission not found"))?;
                if operation == "cancel" {
                    let command =
                        serde_json::from_value(body["command_id"].clone()).map_err(|_| {
                            AgentLoopError::config("stable cancellation identity required")
                        })?;
                    admission.cancel(command).await?;
                    return Ok(json!({"status":"accepted","message_id":message}));
                }
                Ok(json!(admission.state().await?))
            }
            "cursor" => {
                let event = body["event_id"]
                    .as_str()
                    .ok_or_else(|| AgentLoopError::config("event identity required"))?;
                let sequence = handle
                    .event_sequence(event)
                    .await
                    .map_err(|e| AgentLoopError::Internal(anyhow::Error::new(e)))?
                    .ok_or_else(|| {
                        AgentLoopError::config("event cursor does not belong to this session")
                    })?;
                Ok(json!({"sequence":sequence}))
            }
            "list_events" => {
                // Preserve the native HTTP last-N contract with bounded memory,
                // even when filters require scanning a whole pinned snapshot.
                let limit = body["limit"].as_u64().unwrap_or(1000) as usize;
                if !(1..=1000).contains(&limit) {
                    return Err(AgentLoopError::config(
                        "event limit must be between 1 and 1000",
                    ));
                }
                let after = body["after"].as_i64().unwrap_or(0);
                let before = body["before"].as_i64().unwrap_or(i64::MAX);
                if after < 0 || before < 0 {
                    return Err(AgentLoopError::config("negative event sequence"));
                }
                let types: Vec<String> =
                    serde_json::from_value(body.get("types").cloned().unwrap_or(json!([])))
                        .map_err(|_| AgentLoopError::config("invalid event types"))?;
                let exclude: Vec<String> =
                    serde_json::from_value(body.get("exclude").cloned().unwrap_or(json!([])))
                        .map_err(|_| AgentLoopError::config("invalid event exclusion"))?;
                let mut cursor = None;
                let mut events = std::collections::VecDeque::with_capacity(limit);
                let mut total = 0;
                let mut bytes = 0usize;
                loop {
                    let page = handle
                        .event_page(cursor, 256)
                        .await
                        .map_err(|e| AgentLoopError::Internal(anyhow::Error::new(e)))?;
                    total += page.events.len();
                    for event in page.events {
                        let sequence = i64::from(event.sequence().unwrap_or(0));
                        if sequence > after
                            && sequence < before
                            && (types.is_empty()
                                || types.iter().any(|kind| kind == event.event_type()))
                            && !exclude.iter().any(|kind| kind == event.event_type())
                        {
                            if events.len() == limit
                                && let Some((_, size)) = events.pop_front()
                            {
                                bytes -= size;
                            }
                            let value =
                                everruns::execution_api::public_event(event.canonical_json());
                            let size = serde_json::to_vec(&value)
                                .map_err(|_| AgentLoopError::store("invalid canonical event"))?
                                .len();
                            bytes += size;
                            if bytes > 4 * 1024 * 1024 {
                                return Err(AgentLoopError::Internal(anyhow::Error::new(
                                    QueryBudgetExceeded,
                                )));
                            }
                            events.push_back((value, size));
                        }
                    }
                    match page.next_cursor {
                        Some(next) => cursor = Some(next),
                        None => break,
                    }
                }
                Ok(
                    json!({"data":events.into_iter().map(|(value,_)|value).collect::<Vec<_>>(),"total":total}),
                )
            }
            "events" => {
                let cursor: Option<SessionEventCursor> = body
                    .get("cursor")
                    .filter(|value| !value.is_null())
                    .map(|value| serde_json::from_value(value.clone()))
                    .transpose()
                    .map_err(|_| AgentLoopError::config("invalid event cursor"))?;
                let cursor = match cursor {
                    Some(cursor) => Some(cursor),
                    None => Some(
                        SessionEventCursor::after(
                            session,
                            body["after"]
                                .as_i64()
                                .unwrap_or(0)
                                .try_into()
                                .map_err(|_| AgentLoopError::config("invalid event sequence"))?,
                        )
                        .map_err(|e| AgentLoopError::config(e.to_string()))?,
                    ),
                };
                let limit = body["limit"]
                    .as_u64()
                    .unwrap_or(256)
                    .try_into()
                    .map_err(|_| AgentLoopError::config("invalid event page limit"))?;
                let page = handle
                    .event_page(cursor, limit)
                    .await
                    .map_err(|e| AgentLoopError::Internal(anyhow::Error::new(e)))?;
                Ok(
                    json!({"events":page.events.iter().map(|event|everruns::execution_api::public_event(event.canonical_json())).collect::<Vec<_>>(),"next_cursor":page.next_cursor,"high_watermark":page.high_watermark}),
                )
            }
            "binding" => Ok(json!(self.engine.session_binding(session).await?)),
            "pending" => {
                Ok(json!(handle.pending_tools().await.map_err(|e| {
                    AgentLoopError::Internal(anyhow::Error::new(e))
                })?))
            }
            "answer_questions" => {
                let request: everruns::hosting::QuestionAnswersRequest =
                    serde_json::from_value(body)
                        .map_err(|error| AgentLoopError::config(error.to_string()))?;
                let status = request.status;
                handle
                    .answer_questions(request)
                    .await
                    .map_err(|error| AgentLoopError::Internal(anyhow::Error::new(error)))?;
                let phase = self.engine.session_status(session).await?;
                Ok(json!({"status":status,"answered_by":"user","session_status":phase.phase()}))
            }
            "submit_approvals" => {
                let request: everruns::execution_api::SubmitToolApprovalsCommand =
                    serde_json::from_value(body)
                        .map_err(|error| AgentLoopError::config(error.to_string()))?;
                if request.command_id.is_some() && request.turn_id.is_none() {
                    return Err(AgentLoopError::config("command_id requires turn_id"));
                }
                let target = match request.turn_id {
                    Some(turn) => turn,
                    None => self
                        .engine
                        .session_status(session)
                        .await?
                        .open_turn
                        .ok_or_else(|| AgentLoopError::config("no open turn"))?,
                };
                let result = handle
                    .submit_approvals(
                        request.command_id.unwrap_or_else(uuid::Uuid::new_v4),
                        target,
                        request.request.decisions,
                    )
                    .await
                    .map_err(|error| AgentLoopError::Internal(anyhow::Error::new(error)))?;
                Ok(json!(result))
            }
            "approve" => {
                let command = serde_json::from_value(body["command_id"].clone())
                    .map_err(|_| AgentLoopError::config("stable approval identity required"))?;
                let request = body["request_id"]
                    .as_str()
                    .ok_or_else(|| AgentLoopError::config("approval request identity required"))?;
                let decision = serde_json::from_value(body["decision"].clone())
                    .map_err(|_| AgentLoopError::config("invalid approval decision"))?;
                handle
                    .approve_tool(
                        command,
                        serde_json::from_value(body["turn_id"].clone())
                            .map_err(|_| AgentLoopError::config("target turn identity required"))?,
                        request,
                        decision,
                    )
                    .await
                    .map_err(|error| AgentLoopError::Internal(anyhow::Error::new(error)))?;
                Ok(json!({"status":"accepted"}))
            }
            "resolve" => {
                let command = serde_json::from_value(body["command_id"].clone())
                    .map_err(|_| AgentLoopError::config("stable resolution identity required"))?;
                let results = serde_json::from_value(body["results"].clone())
                    .map_err(|_| AgentLoopError::config("invalid client tool results"))?;
                handle
                    .resolve_tools(
                        command,
                        serde_json::from_value(body["turn_id"].clone())
                            .map_err(|_| AgentLoopError::config("target turn identity required"))?,
                        results,
                    )
                    .await
                    .map_err(|e| AgentLoopError::Internal(anyhow::Error::new(e)))?;
                Ok(json!({"status":"accepted"}))
            }
            _ => Err(AgentLoopError::config("unsupported session API operation")),
        }
    }
}
