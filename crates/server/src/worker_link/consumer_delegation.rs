//! A Work Chat consumer may run the publisher's configured workers, never
//! management commands. This bounded adapter is shared by local and RPC workers.
use crate::domains::common::CommandError;
use crate::domains::sessions::{SessionService, types::CreateSessionRequest};
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use serde_json::{Value, json};
use std::sync::Arc;

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
    let Some(channel) = session.channel_id else {
        return Ok(None);
    };
    let Some(channel_id) = db.get_agent_channel_public_id(org, channel).await? else {
        return Ok(None);
    };
    let Some((context, channel)) =
        crate::api::channel_ingress::resolve_channel(db, encryption, &channel_id).await?
    else {
        return Ok(None);
    };
    if channel.channel_type != crate::domains::agent_channels::record::ChannelType::WorkChat {
        return Ok(None);
    };
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
    let agent = crate::domains::agents::queries::resolve(db, org, &root_agent.to_string())
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    let harness = crate::domains::harnesses::queries::resolve_effective(db, org, agent.harness_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Harness"))?;
    if !crate::domains::agent_channels::work_chat::features(db, org, root_agent, context.harness_id)
        .await?
        .threads
    {
        return Err(CommandError::forbidden("Coordinator tools unavailable"));
    }
    let registry = crate::platform::oss_capability_registry();
    let mut capabilities = harness.capabilities;
    capabilities.extend(agent.capabilities);
    let capabilities =
        everruns_core::capabilities::resolve_capability_configs(&capabilities, &registry)
            .map_err(|e| CommandError::bad_request(e.to_string()))?;
    let coordination = capabilities
        .iter()
        .find(|cap| cap.capability_id() == "coordination")
        .ok_or_else(|| CommandError::forbidden("Coordinator capability required"))?;
    if coordination
        .config_value()
        .get("role")
        .and_then(Value::as_str)
        == Some("worker")
    {
        return Err(CommandError::forbidden("Coordinator required"));
    }
    let workers = coordination
        .config_value()
        .get("workers")
        .and_then(Value::as_array);
    let allowed = |id: AgentId| {
        workers.map_or(id == root_agent, |workers| {
            workers.iter().any(|worker| match worker["id"].as_str() {
                Some("self") => id == root_agent,
                Some("any") => true,
                Some(worker) => worker == id.to_string(),
                None => false,
            })
        })
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
            if target.channel_id != Some(channel.internal_id)
                || target.owner_principal_id != owner.principal_id
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
                                runtime_subject_principal_id: Some(owner.principal_id),
                                event_metadata: None,
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
                .create_from_app(
                    &everruns_core::Caller::internal(org),
                    target.harness_id.uuid(),
                    Some(target.internal_id),
                    Some(target.public_id),
                    root.app_id,
                    Some(channel.internal_id),
                    None,
                    owner.principal_id,
                    None,
                    crate::domains::sessions::record::SessionSource::Subagent,
                    request,
                )
                .await?;
            serde_json::to_value(child.execution_session()).map_err(anyhow::Error::from)?
        }
        _ => {
            return Err(CommandError::forbidden(
                "Consumer runtime cannot execute management commands",
            ));
        }
    };
    Ok(Some(
        serde_json::to_string(&response).map_err(anyhow::Error::from)?,
    ))
}
