use super::*;
use crate::api::tool_approvals::{
    ApprovalOutcome, ApprovalServices, ToolApprovalAnswer, ToolApprovalDecision,
    pending_tool_approvals, resolve_tool_approvals, validate_decisions,
};

#[derive(Deserialize)]
struct Binding {
    task: Uuid,
    request: String,
    fingerprint: String,
}

/// Called only after Slack signature verification. Acknowledge after durable
/// intake; task locks, engine resolution and card updates run off the webhook.
pub(crate) async fn enqueue(
    state: &SlackState,
    endpoint: &IngressChannel,
    config: &SlackChannelConfig,
    payload: &Value,
) -> anyhow::Result<bool> {
    let action = &payload["actions"][0];
    let name = action["action_id"].as_str().unwrap_or_default();
    if !matches!(name, "everruns_code_allow" | "everruns_code_reject") {
        return Ok(false);
    }
    if !config.code_channels_enabled || payload["team"]["id"].as_str() != config.team_id.as_deref()
    {
        return Ok(true);
    }
    let Some(team) = config.team_id.as_deref() else {
        return Ok(true);
    };
    let Some(user) = payload["user"]["id"].as_str() else {
        return Ok(true);
    };
    let Some(raw) = action["value"].as_str() else {
        return Ok(true);
    };
    let Ok(binding) = serde_json::from_str::<Binding>(raw) else {
        return Ok(true);
    };
    let key = stable_id(endpoint.internal_id, team, user, &format!("{name}:{raw}"));
    // THREAT[TM-SLACK-016]: intake grants no authority. The worker revalidates
    // the current installation, requester, exact pending request and deadline.
    sqlx::query("INSERT INTO slack_code_inbox (id,endpoint_id,team_id,event_key,payload) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (endpoint_id,team_id,event_key) DO NOTHING")
        .bind(Uuid::now_v7()).bind(endpoint.internal_id).bind(team)
        .bind(format!("approval:{}:{key}",binding.task))
        // Persist authority coordinates only, never Slack's credential-bearing
        // response_url, trigger tokens or the original message's full contents.
        .bind(json!({"type":"code_approval","interaction":{
            "actions":[{"action_id":name,"value":raw}],
            "team":{"id":team},"user":{"id":user},
            "channel":{"id":payload["channel"]["id"]},
            "message":{"ts":payload["message"]["ts"]}
        }}))
        .execute(state.db.pool()).await?;
    Ok(true)
}

pub(crate) async fn handle(
    state: &SlackState,
    endpoint: &IngressChannel,
    config: &SlackChannelConfig,
    payload: &Value,
) -> anyhow::Result<bool> {
    let action = &payload["actions"][0];
    let name = action["action_id"].as_str().unwrap_or_default();
    if !matches!(name, "everruns_code_allow" | "everruns_code_reject") {
        return Ok(false);
    }
    if !config.code_channels_enabled {
        return Ok(true);
    }
    let Some(raw) = action["value"].as_str() else {
        return Ok(true);
    };
    let Ok(binding) = serde_json::from_str::<Binding>(raw) else {
        return Ok(true);
    };
    let mut tx = everruns_durable::begin_transaction(state.db.pool()).await?;
    let Some(task) = sqlx::query_as::<_, Task>(
        "SELECT * FROM slack_code_tasks WHERE id=$1 AND endpoint_id=$2 FOR UPDATE",
    )
    .bind(binding.task)
    .bind(endpoint.internal_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(true);
    };
    if store::stopped_at(state, &task).await?.is_some() {
        return Ok(true);
    }
    if task.installation != installation(config)
        || task.stopped
        || task.archived
        || task.disabled
        || config.team_id.as_deref() != Some(&task.team_id)
        || payload["user"]["id"].as_str() != Some(&task.requester)
        || payload["team"]["id"].as_str() != Some(&task.team_id)
        || payload["channel"]["id"].as_str() != task.slack_channel.as_deref()
    {
        return Ok(true);
    }
    let session = SessionId::from_uuid(task.session_id.context("Approval session missing")?);
    // THREAT[TM-SLACK-016]: only the requester can resolve the exact live gate.
    let pending = pending_tool_approvals(&state.db, session).await?;
    let Some(request) = pending.iter().find(|p| {
        p.request_call_id == binding.request && p.request.fingerprint == binding.fingerprint
    }) else {
        return Ok(true);
    };
    if request
        .expires_at
        .is_none_or(|deadline| deadline <= chrono::Utc::now())
    {
        return Ok(true);
    }
    let decision = if name == "everruns_code_allow" {
        ToolApprovalDecision::Allow
    } else {
        ToolApprovalDecision::Reject
    };
    let outcomes = match validate_decisions(
        &pending,
        &[ToolApprovalAnswer {
            tool_call_id: binding.request,
            decision,
        }],
        chrono::Utc::now(),
    ) {
        Ok(value) => value,
        Err(_) => return Ok(true),
    };
    let result = resolve_tool_approvals(
        &ApprovalServices {
            db: &state.db,
            event_service: &state.event_service,
            runner: state.message_service.runner(),
            credential_authorizer: None,
            external_actor: Some(ExternalActor {
                actor_id: task.requester.clone(),
                actor_name: None,
                source: "slack".into(),
                metadata: Some(std::collections::HashMap::from([(
                    "team_id".into(),
                    task.team_id.clone(),
                )])),
            }),
        },
        task.org_id,
        session,
        &pending,
        &outcomes,
        ApprovalOutcome::NotApproved,
        &format!("slack_code:{}:{}", task.team_id, task.requester),
    )
    .await;
    match result {
        Ok(_) => {
            if store::stopped_at(state, &task).await?.is_some() {
                worker::cancel(state, session.uuid()).await?;
                return Ok(true);
            }
            // Replays cannot resolve again even if updating the visual card fails.
            if store::stopped_at(state, &task).await?.is_none()
                && let Some(ts) = payload["message"]["ts"].as_str()
            {
                let _=slack_api_call(&state.code_api_base,&config.bot_token,"chat.update",json!({"channel":task.slack_channel,"ts":ts,"text":"Approval recorded","blocks":[]})).await;
            }
        }
        Err(error) => tracing::warn!(?error, "Slack Code approval refused"),
    }
    everruns_durable::commit_transaction(tx).await?;
    Ok(true)
}
