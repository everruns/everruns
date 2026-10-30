use super::*;
use everruns_core::channel::SessionBinding;
use everruns_core::channel_runtime::ChannelScope;

struct Admission {
    config: everruns_core::channel_runtime::ChannelConfig,
    message: everruns_core::InputMessage,
    task: Task,
}

pub(super) async fn inbox_step(state: &SlackState) -> anyhow::Result<bool> {
    let mut tx = everruns_durable::begin_transaction(state.db.pool()).await?;
    let Some(inbox) = sqlx::query_as::<_, Inbox>("SELECT * FROM slack_code_inbox WHERE NOT done AND next_attempt<=NOW() ORDER BY next_attempt,id LIMIT 1 FOR UPDATE SKIP LOCKED")
        .fetch_optional(&mut *tx).await? else { return Ok(false); };
    let result = process(state, &inbox, &mut tx).await;
    match result {
        Ok(admission) => {
            sqlx::query("UPDATE slack_code_inbox SET done=$2,next_attempt=NOW()+INTERVAL '30 seconds' WHERE id=$1")
                .bind(inbox.id).bind(admission.is_none()).execute(&mut *tx).await?;
            everruns_durable::commit_transaction(tx).await?;
            if let Some(admission) = admission {
                if store::stopped_at(state, &admission.task).await?.is_some() {
                    sqlx::query("UPDATE slack_code_inbox SET done=TRUE WHERE id=$1")
                        .bind(inbox.id)
                        .execute(state.db.pool())
                        .await?;
                    return Ok(true);
                }
                let port = ServerChannelPort::new(
                    state.db.clone(),
                    state.session_service.clone(),
                    state.message_service.clone(),
                );
                let session = SessionId::from_uuid(
                    admission
                        .task
                        .session_id
                        .context("Missing admission session")?,
                );
                match port
                    .send(&admission.config, &session.to_string(), admission.message)
                    .await
                {
                    Ok(_) => {
                        // A stop can arrive between the pre-admission check and
                        // message commit. Cancel that newly admitted turn too.
                        if store::stopped_at(state, &admission.task).await?.is_some() {
                            cancel(state, session.uuid()).await?;
                        }
                        sqlx::query("UPDATE slack_code_inbox SET done=TRUE WHERE id=$1")
                            .bind(inbox.id)
                            .execute(state.db.pool())
                            .await?;
                        if let Some(scope) = admission.config.scope.get::<ServerChannelScope>() {
                            let config: SlackChannelConfig =
                                serde_json::from_value(scope.channel.channel_config.clone())?;
                            if config.first_message_received_at.is_none() {
                                let _=crate::domains::agent_channels::slack_evidence::record(&state.db,state.encryption.as_ref(),scope.channel.internal_id,&scope.channel.public_id.to_string(),&config.signing_secret,crate::domains::agent_channels::slack_evidence::DeliveryEvidence::FirstMessage).await;
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(id=%inbox.id,%error,"Slack Code admission retained for retry")
                    }
                }
            }
        }
        Err(error) => {
            tx.rollback().await?;
            if error
                .downcast_ref::<SlackApiError>()
                .is_some_and(|e| matches!(e, SlackApiError::Permanent(_)))
            {
                sqlx::query("UPDATE slack_code_tasks SET disabled=TRUE,stopped=TRUE WHERE endpoint_id=$1 AND team_id=$2 AND origin_channel=$3 AND origin_ts=$4")
                    .bind(inbox.endpoint_id).bind(&inbox.team_id).bind(inbox.payload["channel"].as_str()).bind(inbox.payload["ts"].as_str()).execute(state.db.pool()).await?;
                sqlx::query("UPDATE slack_code_inbox SET done=TRUE WHERE id=$1")
                    .bind(inbox.id)
                    .execute(state.db.pool())
                    .await?;
                return Ok(true);
            }
            // Bound retries without storing API errors that might expose content.
            sqlx::query(
                "UPDATE slack_code_inbox SET next_attempt=NOW()+INTERVAL '30 seconds' WHERE id=$1",
            )
            .bind(inbox.id)
            .execute(state.db.pool())
            .await?;
            tracing::warn!(id=%inbox.id, %error, "Slack Code input retained for retry");
        }
    }
    Ok(true)
}

async fn process(
    state: &SlackState,
    inbox: &Inbox,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> anyhow::Result<Option<Admission>> {
    let (mut resolved, config) = match endpoint(state, inbox.endpoint_id).await {
        Ok(value) => value,
        Err(error) if error.is::<EndpointUnavailable>() => {
            sqlx::query(
                "UPDATE slack_code_tasks SET disabled=TRUE,stopped=TRUE WHERE endpoint_id=$1",
            )
            .bind(inbox.endpoint_id)
            .execute(&mut **tx)
            .await?;
            let sessions: Vec<Uuid> = sqlx::query_scalar("SELECT session_id FROM slack_code_tasks WHERE endpoint_id=$1 AND session_id IS NOT NULL")
                .bind(inbox.endpoint_id).fetch_all(&mut **tx).await?;
            for session in sessions {
                cancel(state, session).await?;
            }
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if config.team_id.as_deref() != Some(&inbox.team_id) {
        return Ok(None);
    }
    if inbox.payload["type"] == "code_approval" {
        let scope = resolved
            .config
            .scope
            .get::<ServerChannelScope>()
            .context("Missing approval endpoint")?;
        interaction::handle(
            state,
            &scope.channel,
            &config,
            &inbox.payload["interaction"],
        )
        .await?;
        return Ok(None);
    }
    let event = &inbox.payload;
    let kind = event["type"].as_str().unwrap_or_default();
    if matches!(kind, "app_uninstalled" | "tokens_revoked") {
        sqlx::query("UPDATE slack_code_tasks SET disabled=TRUE,stopped=TRUE,stop_at=NOW() WHERE endpoint_id=$1 AND team_id=$2")
            .bind(inbox.endpoint_id).bind(&inbox.team_id).execute(&mut **tx).await?;
        let sessions: Vec<Uuid> = sqlx::query_scalar("SELECT session_id FROM slack_code_tasks WHERE endpoint_id=$1 AND session_id IS NOT NULL")
            .bind(inbox.endpoint_id).fetch_all(&mut **tx).await?;
        for session in sessions {
            cancel(state, session).await?;
        }
        return Ok(None);
    }
    // THREAT[TM-SLACK-014]: all routing stays within the signed endpoint/workspace.
    let channel = event["channel"].as_str().context("Missing Slack channel")?;
    let ts = event["ts"].as_str().unwrap_or_default();
    let root = event["thread_ts"].as_str().unwrap_or(ts);
    let mut task = store::find_task(
        state.db.pool(),
        inbox.endpoint_id,
        &inbox.team_id,
        channel,
        root,
    )
    .await?;
    if task.is_none() && kind == "agent_session_stopped" {
        // Creation may have succeeded remotely before its local commit. Recover
        // the mapping only from authenticated, assigned channel properties.
        let info = channel_info(&state.code_api_base, &config, &inbox.team_id, channel).await?;
        if !is_code(&info) || !scope_origin(&config, &info, channel) {
            return Ok(None);
        }
        let link = &info["properties"]["agent_session"]["origin_link"];
        if let (Some(origin), Some(ts)) = (link["channel_id"].as_str(), link["ts"].as_str()) {
            task = store::find_task(
                state.db.pool(),
                inbox.endpoint_id,
                &inbox.team_id,
                origin,
                ts,
            )
            .await?;
            if let Some(task) = &task {
                sqlx::query("UPDATE slack_code_tasks SET slack_channel=$2,stopped=TRUE,stop_at=NOW() WHERE id=$1")
                    .bind(task.id).bind(channel).execute(state.db.pool()).await?;
            }
        }
    }
    if task.is_none() {
        if !matches!(kind, "app_mention" | "message") {
            return Ok(None);
        }
        let info = channel_info(&state.code_api_base, &config, &inbox.team_id, channel).await?;
        if kind == "message" && !is_code(&info) {
            return Ok(None);
        }
        if !scope_origin(&config, &info, channel) {
            return Ok(None);
        }
        // A new task gets only this message, never a previous DM/thread history.
        let origin_ts = if is_code(&info) { "code-channel" } else { ts };
        if origin_ts.is_empty() {
            return Ok(None);
        }
        let linked = if is_code(&info) {
            let link = &info["properties"]["agent_session"]["origin_link"];
            match (link["channel_id"].as_str(), link["ts"].as_str()) {
                (Some(origin), Some(ts)) => {
                    store::find_task(
                        state.db.pool(),
                        inbox.endpoint_id,
                        &inbox.team_id,
                        origin,
                        ts,
                    )
                    .await?
                }
                _ => None,
            }
        } else {
            None
        };
        let id = linked
            .as_ref()
            .map(|t| t.id)
            .unwrap_or_else(|| stable_id(inbox.endpoint_id, &inbox.team_id, channel, origin_ts));
        let requester = event["user"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Missing requester")?;
        let scope = resolved
            .config
            .scope
            .get::<ServerChannelScope>()
            .context("Missing scope")?;
        sqlx::query("INSERT INTO slack_code_tasks (id,endpoint_id,org_id,team_id,origin_channel,origin_ts,requester,slack_channel) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING")
            .bind(id).bind(inbox.endpoint_id).bind(scope.context.org_id).bind(&inbox.team_id)
            .bind(channel).bind(ts).bind(requester).bind(is_code(&info).then_some(channel))
            .execute(state.db.pool()).await?;
        task = sqlx::query_as("SELECT * FROM slack_code_tasks WHERE id=$1")
            .bind(id)
            .fetch_optional(state.db.pool())
            .await?;
    }
    let Some(task) = task else {
        return Ok(None);
    };
    let mut task: Task = sqlx::query_as("SELECT * FROM slack_code_tasks WHERE id=$1 FOR UPDATE")
        .bind(task.id)
        .fetch_one(&mut **tx)
        .await?;
    if task.disabled {
        return Ok(None);
    }
    if task.session_id.is_none() {
        let streaming = ResolvedChannel::streaming(resolved.config.clone(), "slack");
        let session = state
            .channels
            .as_ref()
            .context("Missing host")?
            .host
            .conversation(
                &streaming,
                &store::binding_key(&task),
                event["text"].as_str().unwrap_or("Slack Code task"),
            )
            .await?;
        task.session_id = Some(session.session_id.parse::<SessionId>()?.uuid());
        sqlx::query("UPDATE slack_code_tasks SET session_id=$2 WHERE id=$1")
            .bind(task.id)
            .bind(task.session_id)
            .execute(&mut **tx)
            .await?;
    }
    let session = task.session_id.context("Missing session")?;
    if task.slack_channel.is_none() {
        // The stable local binding above is committed independently by the host.
        // Never supply is_private: Slack must inherit the origin's privacy.
        let response = slack_api_call(
            &state.code_api_base,
            &config.bot_token,
            "agents.conversations.create",
            json!({
                "session_id":SessionId::from_uuid(session).to_string(),
                "origin_channel_id":task.origin_channel,"origin_message_ts":task.origin_ts,
                "team_id":task.team_id,
            }),
        )
        .await?;
        let created = response["channel_id"]
            .as_str()
            .context("Creation returned no channel")?;
        task.slack_channel = Some(created.to_string());
        sqlx::query("UPDATE slack_code_tasks SET slack_channel=$2 WHERE id=$1")
            .bind(task.id)
            .bind(created)
            .execute(&mut **tx)
            .await?;
    }
    let code_channel = task
        .slack_channel
        .as_deref()
        .context("Missing code channel")?;
    if config.team_id.as_deref() != Some(&task.team_id) {
        return Ok(None);
    }
    if task.installation != installation(&config) {
        let info = channel_info(&state.code_api_base, &config, &task.team_id, code_channel).await?;
        if !is_code(&info) {
            bail!("Bound conversation is no longer a code channel")
        }
        sqlx::query("UPDATE slack_code_tasks SET installation=$2 WHERE id=$1")
            .bind(task.id)
            .bind(installation(&config))
            .execute(&mut **tx)
            .await?;
        task.installation = installation(&config);
    }
    if kind == "agent_session_stopped" {
        sqlx::query(
            "UPDATE slack_code_tasks SET stopped=TRUE,lifecycle='active',ready=FALSE,stop_at=COALESCE(stop_at,NOW()) WHERE id=$1",
        )
        .bind(task.id)
        .execute(&mut **tx)
        .await?;
        cancel(state, session).await?;
        return Ok(None);
    }
    if matches!(kind, "channel_archive" | "group_archive") {
        sqlx::query("UPDATE slack_code_tasks SET archived=TRUE,stopped=TRUE WHERE id=$1")
            .bind(task.id)
            .execute(&mut **tx)
            .await?;
        cancel(state, session).await?;
        return Ok(None);
    }
    if matches!(kind, "channel_unarchive" | "group_unarchive") {
        let info = channel_info(&state.code_api_base, &config, &task.team_id, code_channel).await?;
        if !is_code(&info) || info["is_archived"] == true {
            return Ok(None);
        }
        sqlx::query("UPDATE slack_code_tasks SET archived=FALSE,stopped=FALSE,stop_at=NULL,summary_ts=NULL,next_attempt=NOW(),desired='{}',version=version+1 WHERE id=$1")
            .bind(task.id).execute(&mut **tx).await?;
        sqlx::query(
            "DELETE FROM slack_code_stops WHERE endpoint_id=$1 AND team_id=$2 AND slack_channel=$3",
        )
        .bind(task.endpoint_id)
        .bind(&task.team_id)
        .bind(code_channel)
        .execute(&mut **tx)
        .await?;
        return Ok(None);
    }
    if kind == "agent_session_title_changed" {
        let title = event["title"].as_str().unwrap_or_default().trim();
        if !title.is_empty() {
            let port = ServerChannelPort::new(
                state.db.clone(),
                state.session_service.clone(),
                state.message_service.clone(),
            );
            port.control(
                &resolved.config,
                &SessionId::from_uuid(session).to_string(),
                &everruns_core::channel::ControlAction::Rename {
                    title: title.into(),
                },
            )
            .await?;
        }
        return Ok(None);
    }
    if task.archived || !matches!(kind, "message" | "app_mention") {
        return Ok(None);
    }
    // A new human input explicitly resumes a stopped task. Advance past the
    // cancelled run so its unsent output cannot be resurrected after restart.
    let stopped_at = store::stopped_at(state, &task).await?.or(task.stop_at);
    if task.stopped || stopped_at.is_some() {
        if stopped_at.is_some_and(|stop| inbox.created_at <= stop) {
            return Ok(None);
        }
        cancel(state, session).await?;
        let cursor: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence),0)::BIGINT FROM events WHERE session_id=$1",
        )
        .bind(session)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM slack_code_stops WHERE endpoint_id=$1 AND team_id=$2 AND slack_channel=$3 AND stopped_at<$4")
            .bind(task.endpoint_id).bind(&task.team_id).bind(code_channel).bind(inbox.created_at).execute(&mut **tx).await?;
        let newer_stop:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM slack_code_stops WHERE endpoint_id=$1 AND team_id=$2 AND slack_channel=$3 AND stopped_at>=$4)")
            .bind(task.endpoint_id).bind(&task.team_id).bind(code_channel).bind(inbox.created_at).fetch_one(&mut **tx).await?;
        if newer_stop {
            return Ok(None);
        }
        sqlx::query("UPDATE slack_code_tasks SET stopped=FALSE,lifecycle='active',ready=FALSE,stop_at=NULL,cursor=$2,next_attempt=NOW(),desired=jsonb_set(desired,'{complete}','false'),version=version+1 WHERE id=$1").bind(task.id).bind(cursor).execute(&mut **tx).await?;
        task.version += 1;
    }
    let original_key = format!("message:{}:{}", task.origin_channel, task.origin_ts);
    let this_key = format!("message:{channel}:{ts}");
    if this_key != original_key {
        let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM slack_code_inbox WHERE endpoint_id=$1 AND team_id=$2 AND event_key=$3 AND NOT done)")
            .bind(task.endpoint_id).bind(&task.team_id).bind(original_key).fetch_one(state.db.pool()).await?;
        if pending {
            bail!("Waiting for the initiating task input")
        }
    }
    let user = event["user"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("Missing human sender")?;
    let actor = ExternalActor {
        actor_id: user.into(),
        actor_name: None,
        source: "slack".into(),
        metadata: Some(std::collections::HashMap::from([
            ("channel".into(), code_channel.into()),
            ("team_id".into(), task.team_id.clone()),
        ])),
    };
    let slack_event: super::super::events::SlackEvent = serde_json::from_value(event.clone())?;
    let inbound = super::super::events::parse_slack_inbound_event(&slack_event, &config, None);
    let id = stable_id(task.endpoint_id, &task.team_id, channel, ts);
    let mut input = everruns_core::InputMessage::user(inbound.text)
        .with_external_actor(actor)
        .with_metadata("slack_code_message_id", json!(id.to_string()))
        .with_metadata("slack_code_task", json!(true))
        .with_metadata("slack_code_revision", json!(task.version))
        .with_metadata(
            "_app_channel_id",
            json!(
                resolved
                    .config
                    .scope
                    .get::<ServerChannelScope>()
                    .context("Missing scope")?
                    .channel
                    .public_id
                    .to_string()
            ),
        )
        .with_metadata("slack_channel", json!(code_channel))
        .with_metadata("slack_thread_ts", json!(""))
        .with_metadata("slack_reply_inline", json!(true));
    let attachments = state
        .channels
        .as_ref()
        .context("Missing host")?
        .host
        .attachment_parts(
            &ResolvedChannel::streaming(resolved.config.clone(), "slack"),
            Some(&SessionId::from_uuid(session).to_string()),
            &inbound.attachments,
            None,
        )
        .await?;
    input.content.extend(attachments);
    if channel == code_channel {
        input = input.with_metadata("slack_ts", json!(ts));
    }
    let mut scope = resolved
        .config
        .scope
        .get::<ServerChannelScope>()
        .context("Missing scope")?
        .clone();
    // Track the channel as one conversation, including inputs posted in threads.
    scope.participant_thread_key = Some("slack_code_thread");
    input = input.with_metadata("slack_code_thread", json!(format!("code:{code_channel}")));
    resolved.config.scope = ChannelScope::new(scope);
    resolved.config.binding = SessionBinding::Conversation;
    Ok(Some(Admission {
        config: resolved.config,
        message: input,
        task,
    }))
}

pub(super) async fn cancel(state: &SlackState, session: Uuid) -> anyhow::Result<()> {
    use everruns_core::session_task::SessionTaskRegistry;
    let session = SessionId::from_uuid(session);
    let tasks = crate::storage::DbSessionTaskRegistry::new(state.db.clone())
        .with_event_emitter(state.event_service.clone());
    for task in tasks.list(session, None).await? {
        if !task.state.is_terminal() {
            tasks.request_cancel(session, &task.id).await?;
        }
    }
    crate::api::channel_api::cancel_session_turn_for(
        &state.db,
        &state.message_service,
        session,
        "Slack Code stop",
    )
    .await
}
