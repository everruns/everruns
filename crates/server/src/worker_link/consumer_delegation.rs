//! A Work Chat consumer may run the publisher's configured workers, never
//! management commands. This bounded adapter is shared by local and RPC workers.
//!
//! Decision (2026-11-24): an automated run (a schedule, webhook or event
//! message, carrying no person) of a coordinator agent gets the same bounded
//! commands. Automated messages deliberately carry no management authority,
//! so before this every `start_thread` in a scheduled run was refused. The
//! authority here is the agent's own Threads config, which a manager wrote,
//! never anything in the event: threads run only on the agents it allows
//! (`self`, its named `agents`, or any with `any_agent`), only as children of
//! the run's first session, owned by the same principal.
//!
//! Decision (2026-11-26): a coordinator answering in a Slack conversation (a
//! workspace member's message there) or running a trigger aimed into a channel
//! conversation gets the same bounded commands. Before this, `start_thread`
//! from Slack fell through to the management path and was refused, since a
//! Slack member is never an organization user. The authority is still only
//! the agent's Threads config; the Slack member's message is attributed to
//! them in the thread, never lifted to management authority. Threads of a
//! channel conversation carry no channel, so a worker never posts into the
//! channel itself; only the coordinator answers there.
use crate::domains::common::CommandError;
use crate::domains::sessions::{SessionService, types::CreateSessionRequest};
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::{AgentId, HarnessId, PrincipalId, SessionId};
use serde_json::{Value, json};
use std::sync::Arc;

/// Who a delegated runtime command acts for, and what it may reach.
struct Scope {
    root_id: SessionId,
    root: crate::storage::SessionRow,
    root_agent: AgentId,
    owner: PrincipalId,
    /// The channel threads are bound to: the Work Chat endpoint, `None` for
    /// automated runs and other channel conversations.
    child_channel: Option<uuid::Uuid>,
    /// A run without Threads keeps the management path (which refuses it)
    /// instead of failing here; only Work Chat consumers fail here.
    threads_required: bool,
    /// The end user a consumer message is attributed to; `None` for automated
    /// runs, whose messages stay automated.
    subject: Option<PrincipalId>,
    capabilities: Vec<everruns_contracts::CapabilityRef>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch(
    db: &Arc<StorageBackend>,
    sessions: &SessionService,
    encryption: Option<&Arc<crate::storage::EncryptionService>>,
    messages: Option<&crate::domains::messages::MessageService>,
    org: i64,
    current: SessionId,
    message: uuid::Uuid,
    name: &str,
    params: &Value,
) -> Result<Option<String>, CommandError> {
    let Some(session) = db.get_session(org, current).await? else {
        return Ok(None);
    };
    let scope = match session.channel_id {
        Some(channel) => {
            work_chat_scope(db, encryption, org, current, message, &session, channel).await?
        }
        None => automated_scope(db, org, current, message, &session).await?,
    };
    let Some(scope) = scope else {
        return Ok(None);
    };
    let Scope {
        root_id,
        root,
        root_agent,
        owner,
        child_channel,
        threads_required,
        subject,
        capabilities,
    } = scope;
    let registry = crate::platform::oss_capability_registry();
    let capabilities =
        everruns_core::capabilities::resolve_capability_configs(&capabilities, &registry)
            .map_err(|e| CommandError::bad_request(e.to_string()))?;
    let threads = match capabilities.iter().find(|cap| {
        cap.capability_id() == everruns_capabilities::capabilities::THREADS_CAPABILITY_ID
    }) {
        Some(threads) => threads,
        // An automated run or channel conversation of an agent without
        // Threads keeps the management path, which refuses it as before.
        None if !threads_required => return Ok(None),
        None => return Err(CommandError::forbidden("Threads capability required")),
    };
    let config = threads.config_value();
    let runs_self = config.get("self").and_then(Value::as_bool).unwrap_or(true);
    let any_agent = config
        .get("any_agent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let named: Vec<String> = config
        .get("agents")
        .and_then(Value::as_array)
        .map(|agents| {
            agents
                .iter()
                .filter_map(|agent| agent.get("agent_id").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let allowed = |id: AgentId| {
        (runs_self && id == root_agent) || any_agent || named.iter().any(|n| *n == id.to_string())
    };
    let target_id = params
        .get("agent_id")
        .or_else(|| params.get("id"))
        .and_then(Value::as_str)
        .and_then(|id| id.parse::<AgentId>().ok())
        .unwrap_or(root_agent);
    let response = match name {
        "get_agent" | "get_agent_harness" => {
            if !allowed(target_id) {
                return Err(CommandError::forbidden("Worker not configured"));
            }
            let target = crate::domains::agents::queries::resolve(db, org, &target_id.to_string())
                .await?
                .filter(|a| a.status == crate::domains::agents::record::AgentStatus::Active)
                .ok_or_else(|| CommandError::not_found("Agent"))?;
            if name == "get_agent" {
                serde_json::to_value(target.definition()).map_err(anyhow::Error::from)?
            } else {
                json!(target.harness_id)
            }
        }
        "get_harness" => {
            let id = params["id"]
                .as_str()
                .and_then(|id| id.parse::<HarnessId>().ok())
                .ok_or_else(|| CommandError::not_found("Harness"))?;
            // Harness reads are limited to the current run; target agents carry
            // their own definitions through the worker-loading path.
            if session.harness_id != Some(id) {
                return Err(CommandError::forbidden("Harness outside conversation"));
            }
            let harness = crate::domains::harnesses::queries::resolve_effective(db, org, id)
                .await?
                .ok_or_else(|| CommandError::not_found("Harness"))?;
            serde_json::to_value(harness.definition()).map_err(anyhow::Error::from)?
        }
        "get_session" | "archive_session" | "unarchive_session" | "create_message"
        | "list_messages" => {
            let id = params
                .get("session_id")
                .or_else(|| params.get("id"))
                .and_then(Value::as_str)
                .and_then(|id| id.parse::<SessionId>().ok())
                .ok_or_else(|| CommandError::not_found("Session"))?;
            let target = db
                .get_session(org, id)
                .await?
                .ok_or_else(|| CommandError::not_found("Session"))?;
            let expected_channel = if target.id == current {
                session.channel_id
            } else {
                child_channel
            };
            if target.channel_id != expected_channel
                || target.owner_principal_id != owner
                || !(target.id == current || target.parent_session_id == Some(current))
            {
                return Err(CommandError::forbidden("Session outside conversation"));
            }
            if name == "create_message" || name == "list_messages" {
                let messages =
                    messages.ok_or_else(|| CommandError::forbidden("Messaging unavailable"))?;
                if name == "list_messages" {
                    serde_json::to_value(
                        messages
                            .list_limited(
                                target.id.uuid(),
                                Some(params["limit"].as_i64().unwrap_or(10).clamp(1, 100) as i32),
                            )
                            .await?,
                    )
                    .map_err(anyhow::Error::from)?
                } else {
                    let input: crate::domains::messages::types::InputMessage =
                        serde_json::from_value(params["message"].clone())
                            .map_err(|e| CommandError::bad_request(e.to_string()))?;
                    crate::domains::agent_channels::api_sessions::check_api_message(&input, false)?;
                    let message = messages
                        .create(
                            crate::domains::messages::CreateMessageContext {
                                org_id: org,
                                user_id: None,
                                harness_id: target
                                    .harness_id
                                    .ok_or_else(|| CommandError::not_found("Harness"))?
                                    .uuid(),
                                agent_id: target.agent_id.map(|id| id.uuid()),
                                session_id: target.id.uuid(),
                                runtime_subject_principal_id: subject,
                                // An automated run's relay stays automated: no
                                // person, so no management authority downstream.
                                event_metadata: subject.is_none().then(|| {
                                    json!({"type": "coordination", "session_id": current.to_string()})
                                }),
                                request_id: None,
                            },
                            crate::domains::messages::types::CreateMessageRequest {
                                message: input,
                                addressed_participant_id: None,
                                controls: None,
                                metadata: None,
                                tags: None,
                                external_actor: None,
                                client_message_id: None,
                            },
                        )
                        .await?;
                    serde_json::to_value(message).map_err(anyhow::Error::from)?
                }
            } else if name == "get_session" {
                serde_json::to_value(
                    sessions
                        .get(
                            &everruns_core::Caller::internal(org),
                            target.id.uuid(),
                            None,
                        )
                        .await?
                        .ok_or_else(|| CommandError::not_found("Session"))?
                        .execution_session(),
                )
                .map_err(anyhow::Error::from)?
            } else {
                if id == root_id {
                    return Err(CommandError::forbidden(
                        "Cannot resolve the main conversation",
                    ));
                }
                if name == "archive_session" {
                    sessions
                        .archive(&everruns_core::Caller::internal(org), id.uuid())
                        .await?;
                } else {
                    sessions
                        .unarchive(&everruns_core::Caller::internal(org), id.uuid())
                        .await?;
                }
                json!(true)
            }
        }
        "create_session" => {
            if current != root_id || !allowed(target_id) {
                return Err(CommandError::forbidden("Worker not configured"));
            }
            let parent = params["parent_session_id"]
                .as_str()
                .and_then(|id| id.parse::<SessionId>().ok());
            if parent != Some(current) {
                return Err(CommandError::forbidden(
                    "Threads must belong to this conversation",
                ));
            }
            let target = crate::domains::agents::queries::resolve(db, org, &target_id.to_string())
                .await?
                .filter(|a| a.status == crate::domains::agents::record::AgentStatus::Active)
                .ok_or_else(|| CommandError::not_found("Agent"))?;
            let request: CreateSessionRequest = serde_json::from_value(
                json!({"harness_id":target.harness_id,"agent_id":target.public_id,"title":params["title"],"goal":params["goal"],"parent_session_id":current,"tags":root.tags}),
            ).map_err(|e|CommandError::bad_request(e.to_string()))?;
            let child = sessions
                .create_from_channel(
                    &everruns_core::Caller::internal(org),
                    target.harness_id.uuid(),
                    Some(target.internal_id),
                    Some(target.public_id),
                    child_channel,
                    None,
                    owner,
                    None,
                    crate::domains::sessions::record::SessionSource::Subagent,
                    request,
                )
                .await?;
            serde_json::to_value(child.execution_session()).map_err(anyhow::Error::from)?
        }
        _ => {
            return Err(CommandError::forbidden(
                "Delegated runtime cannot execute management commands",
            ));
        }
    };
    Ok(Some(
        serde_json::to_string(&response).map_err(anyhow::Error::from)?,
    ))
}

/// A channel conversation's scope: a Work Chat consumer's, or another
/// channel's ([`channel_conversation_scope`]). Never management authority.
async fn work_chat_scope(
    db: &Arc<StorageBackend>,
    encryption: Option<&Arc<crate::storage::EncryptionService>>,
    org: i64,
    current: SessionId,
    message: uuid::Uuid,
    session: &crate::storage::SessionRow,
    channel: uuid::Uuid,
) -> Result<Option<Scope>, CommandError> {
    let Some(channel_id) = db.get_agent_channel_public_id(org, channel).await? else {
        return Ok(None);
    };
    let Some((context, channel)) =
        crate::api::channel_ingress::resolve_channel(db, encryption, &channel_id).await?
    else {
        return Ok(None);
    };
    let work_chat =
        channel.channel_type == crate::domains::agent_channels::record::ChannelType::WorkChat;
    if !work_chat {
        return channel_conversation_scope(db, org, current, message, session, &context, &channel)
            .await;
    }
    if crate::api::channel_ingress::channel_liveness(&context, &channel).is_err() {
        return Err(CommandError::forbidden("Channel unavailable"));
    }
    // THREAT[TM-AUTHZ-025]: use the recorded consumer invocation, never management authority.
    let subject = db
        .runtime_invocation_subject(current, message)
        .await?
        .ok_or_else(|| CommandError::forbidden("Consumer invocation required"))?;
    let owner = crate::domains::agent_channels::api_sessions::end_user_principal(
        db, org, subject, "active",
    )
    .await?
    .ok_or_else(|| CommandError::forbidden("Consumer unavailable"))?;
    if session.owner_principal_id != owner.principal_id {
        return Err(CommandError::forbidden("Consumer session mismatch"));
    }
    let root_id = session.root_session_id.unwrap_or(current);
    let root = db
        .get_session(org, root_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Conversation"))?;
    if root.channel_id != Some(channel.internal_id) || root.owner_principal_id != owner.principal_id
    {
        return Err(CommandError::forbidden("Conversation mismatch"));
    }
    let root_agent = context
        .agent_id
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    if !crate::domains::agent_channels::work_chat::features(db, org, root_agent, context.harness_id)
        .await?
        .threads
    {
        return Err(CommandError::forbidden("Coordinator tools unavailable"));
    }
    let capabilities = agent_capabilities(db, org, root_agent).await?;
    Ok(Some(Scope {
        root_id,
        root,
        root_agent,
        owner: owner.principal_id,
        child_channel: Some(channel.internal_id),
        threads_required: true,
        subject: Some(owner.principal_id),
        capabilities,
    }))
}

/// An automated run's scope. THREAT[TM-AUTHZ-028]: only an invocation with no
/// person (no management user, no end user) answered by the session's own
/// agent qualifies; a person's message keeps the management path, and a
/// person's authority is never borrowed. Every command stays inside the run.
async fn automated_scope(
    db: &Arc<StorageBackend>,
    org: i64,
    current: SessionId,
    message: uuid::Uuid,
    session: &crate::storage::SessionRow,
) -> Result<Option<Scope>, CommandError> {
    let Some(agent_id) = session.agent_id else {
        return Ok(None);
    };
    let Some(responder) = db
        .runtime_invocation_automated_responder(current, message)
        .await?
    else {
        return Ok(None);
    };
    let root_id = session.root_session_id.unwrap_or(current);
    let root = db
        .get_session(org, root_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Conversation"))?;
    let Some(root_agent) = root.agent_id else {
        return Ok(None);
    };
    // The coordinator is the run's first agent; its threads relay through it.
    if root.channel_id.is_some() || root.owner_principal_id != session.owner_principal_id {
        return Err(CommandError::forbidden("Conversation mismatch"));
    }
    let agent = crate::domains::agents::queries::resolve(db, org, &agent_id.to_string())
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    if responder != Some(agent.internal_id) {
        return Ok(None);
    }
    let capabilities = agent_capabilities(db, org, root_agent).await?;
    Ok(Some(Scope {
        root_id,
        owner: root.owner_principal_id,
        root,
        root_agent,
        child_channel: None,
        threads_required: false,
        subject: None,
        capabilities,
    }))
}

/// A conversation in a channel other than Work Chat: a Slack member's message,
/// or an automated message (a trigger aimed at the conversation).
/// THREAT[TM-AUTHZ-030]: the session runs the channel's own agent in the channel's
/// own conversation, and the invocation is either a Slack member's (an active
/// end user, attributed to them downstream) or carries no person and is
/// answered by that agent. A management user's invocation keeps the
/// management path. Members of other public channels (public chat, A2A, API,
/// Poppy) get nothing here: they are outside parties, not workspace members.
async fn channel_conversation_scope(
    db: &Arc<StorageBackend>,
    org: i64,
    current: SessionId,
    message: uuid::Uuid,
    session: &crate::storage::SessionRow,
    context: &crate::domains::agent_channels::ingress::IngressContext,
    channel: &crate::domains::agent_channels::ingress::IngressChannel,
) -> Result<Option<Scope>, CommandError> {
    let Some(root_agent) = context.agent_id else {
        return Ok(None);
    };
    if session.agent_id != Some(root_agent) {
        return Ok(None);
    }
    if db
        .runtime_invocation_management_user(current, message)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    let subject = match db.runtime_invocation_subject(current, message).await? {
        Some(subject)
            if channel.channel_type
                == crate::domains::agent_channels::record::ChannelType::Slack =>
        {
            let member = crate::domains::agent_channels::api_sessions::end_user_principal(
                db, org, subject, "active",
            )
            .await?
            .ok_or_else(|| CommandError::forbidden("Member unavailable"))?;
            Some(member.principal_id)
        }
        Some(_) => return Ok(None),
        None => {
            let Some(responder) = db
                .runtime_invocation_automated_responder(current, message)
                .await?
            else {
                return Ok(None);
            };
            if responder != Some(context.agent_internal_id) {
                return Ok(None);
            }
            None
        }
    };
    if crate::api::channel_ingress::channel_liveness(context, channel).is_err() {
        return Err(CommandError::forbidden("Channel unavailable"));
    }
    let root_id = session.root_session_id.unwrap_or(current);
    let root = db
        .get_session(org, root_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Conversation"))?;
    if root.channel_id != Some(channel.internal_id)
        || root.owner_principal_id != session.owner_principal_id
        || root.agent_id != Some(root_agent)
    {
        return Err(CommandError::forbidden("Conversation mismatch"));
    }
    let capabilities = agent_capabilities(db, org, root_agent).await?;
    Ok(Some(Scope {
        root_id,
        owner: root.owner_principal_id,
        root,
        root_agent,
        child_channel: None,
        threads_required: false,
        subject,
        capabilities,
    }))
}

/// The effective capabilities of an agent: its harness's, then its own.
async fn agent_capabilities(
    db: &Arc<StorageBackend>,
    org: i64,
    agent_id: AgentId,
) -> Result<Vec<everruns_contracts::CapabilityRef>, CommandError> {
    let agent = crate::domains::agents::queries::resolve(db, org, &agent_id.to_string())
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    let harness = crate::domains::harnesses::queries::resolve_effective(db, org, agent.harness_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Harness"))?;
    let mut capabilities = harness.capabilities;
    capabilities.extend(agent.capabilities);
    Ok(capabilities)
}
