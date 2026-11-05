//! Durable task authority and signed-event inbox. No bot credentials are stored here.
use super::*;

#[derive(Clone, Debug, sqlx::FromRow)]
pub(super) struct Task {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub org_id: i64,
    pub team_id: String,
    pub origin_channel: String,
    pub origin_ts: String,
    pub requester: String,
    pub installation: String,
    pub slack_channel: Option<String>,
    pub session_id: Option<Uuid>,
    pub stopped: bool,
    pub stop_at: Option<chrono::DateTime<chrono::Utc>>,
    pub archived: bool,
    pub disabled: bool,
    pub cursor: i64,
    pub desired: Value,
    pub version: i64,
    pub published: i64,
    pub published_state: Value,
    pub ready: bool,
    pub lifecycle: String,
    pub summary_ts: Option<String>,
    pub diff_view_id: Option<String>,
    pub diff_file_id: Option<String>,
    pub diff_content_version: Option<i64>,
}

#[derive(sqlx::FromRow)]
pub(super) struct Inbox {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub team_id: String,
    pub payload: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn find_task(
    pool: &sqlx::PgPool,
    endpoint: Uuid,
    team: &str,
    channel: &str,
    thread: &str,
) -> anyhow::Result<Option<Task>> {
    Ok(sqlx::query_as::<_, Task>(
        "SELECT * FROM slack_code_tasks WHERE endpoint_id=$1 AND team_id=$2 AND
         (slack_channel=$3 OR (origin_channel=$3 AND origin_ts=$4)) LIMIT 1",
    )
    .bind(endpoint)
    .bind(team)
    .bind(channel)
    .bind(thread)
    .fetch_optional(pool)
    .await?)
}

/// Called only after signature verification, before acknowledging Slack. The
/// stop flag and queued cancellation commit together, before further delivery.
pub(crate) async fn enqueue(
    state: &SlackState,
    endpoint: &IngressChannel,
    config: &SlackChannelConfig,
    body: &Value,
) -> anyhow::Result<bool> {
    if !config.code_channels_enabled || body["type"] != "event_callback" {
        return Ok(false);
    }
    let Some(team) = body["team_id"]
        .as_str()
        .filter(|t| config.team_id.as_deref() == Some(*t))
    else {
        return Ok(false);
    };
    let mut normalized = body["event"].clone();
    if matches!(
        normalized["type"].as_str(),
        Some("agent_session_stopped" | "agent_session_title_changed")
    ) {
        let parsed: super::super::events::SlackEvent = serde_json::from_value(normalized.clone())?;
        let Some((channel, thread)) = parsed.session_coordinates() else {
            return Ok(true);
        };
        normalized["channel"] = json!(channel);
        if let Some(thread) = thread {
            normalized["thread_ts"] = json!(thread);
        }
        if normalized["title"].is_null() {
            normalized["title"] = json!(
                parsed
                    .assistant_thread
                    .as_ref()
                    .and_then(|t| t.title.as_deref())
            );
        }
    }
    let event = &normalized;
    if matches!(event["channel_type"].as_str(), Some("im" | "mpim")) {
        return Ok(false);
    }
    let kind = event["type"].as_str().unwrap_or_default();
    let channel = event["channel"].as_str().unwrap_or_default();
    let thread = event["thread_ts"]
        .as_str()
        .or(event["ts"].as_str())
        .unwrap_or_default();
    let task = find_task(state.db.pool(), endpoint.internal_id, team, channel, thread).await?;
    let is_mention = kind == "app_mention" && !channel.starts_with('D') && !channel.is_empty();
    let source_message = kind == "message" && !channel.starts_with('D') && !channel.is_empty();
    let unknown_control =
        kind == "agent_session_stopped" && !channel.is_empty() && !channel.starts_with('D');
    if task.is_none()
        && !is_mention
        && !source_message
        && !unknown_control
        && !matches!(kind, "app_uninstalled" | "tokens_revoked")
    {
        return Ok(false);
    }
    if event.get("bot_id").is_some()
        || event["subtype"]
            .as_str()
            .is_some_and(|s| !matches!(s, "file_share" | "thread_broadcast"))
    {
        return Ok(true);
    }
    if !matches!(
        kind,
        "message"
            | "app_mention"
            | "agent_session_stopped"
            | "agent_session_title_changed"
            | "channel_archive"
            | "channel_unarchive"
            | "group_archive"
            | "group_unarchive"
            | "app_uninstalled"
            | "tokens_revoked"
    ) {
        return Ok(true);
    }
    let key = if matches!(kind, "message" | "app_mention") {
        format!(
            "message:{channel}:{}",
            event["ts"].as_str().unwrap_or_default()
        )
    } else {
        format!(
            "{kind}:{}",
            body["event_id"]
                .as_str()
                .or(event["event_ts"].as_str())
                .unwrap_or(thread)
        )
    };
    let mut tx = everruns_durable::begin_transaction(state.db.pool()).await?;
    let inserted: Option<Uuid> = sqlx::query_scalar("INSERT INTO slack_code_inbox (id,endpoint_id,team_id,event_key,payload) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (endpoint_id,team_id,event_key) DO UPDATE SET payload=EXCLUDED.payload,done=FALSE,next_attempt=NOW() WHERE slack_code_inbox.payload->>'type'='message' AND EXCLUDED.payload->>'type'='app_mention' RETURNING id")
        .bind(Uuid::now_v7()).bind(endpoint.internal_id).bind(team).bind(key).bind(event)
        .fetch_optional(&mut *tx).await?;
    if let Some(id) = inserted {
        if kind == "agent_session_stopped" {
            // Origin-thread controls address the same task: stop its bound
            // Code destination before acknowledgment, not only the origin.
            let stop_channel = task
                .as_ref()
                .and_then(|t| t.slack_channel.as_deref())
                .unwrap_or(channel);
            sqlx::query("INSERT INTO slack_code_stops (id,endpoint_id,team_id,slack_channel) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING")
                .bind(id).bind(endpoint.internal_id).bind(team).bind(stop_channel).execute(&mut *tx).await?;
        } else if matches!(kind, "app_uninstalled" | "tokens_revoked") {
            sqlx::query("INSERT INTO slack_code_stops (id,endpoint_id,team_id,slack_channel) SELECT $1,endpoint_id,team_id,slack_channel FROM slack_code_tasks WHERE endpoint_id=$2 AND team_id=$3 AND slack_channel IS NOT NULL ON CONFLICT DO NOTHING")
                .bind(id).bind(endpoint.internal_id).bind(team).execute(&mut *tx).await?;
        }
    }
    everruns_durable::commit_transaction(tx).await?;
    if inserted.is_some()
        && kind == "agent_session_stopped"
        && let Some(session) = task.and_then(|t| t.session_id)
    {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = super::worker::cancel(&state, session).await {
                tracing::warn!(%error,"Immediate Code stop deferred to durable inbox");
            }
        });
    }
    Ok(true)
}

pub(super) fn binding_key(task: &Task) -> String {
    format!("slack:code:{}:{}", task.team_id, task.id)
}

/// Only the bound session's control-plane invoker can change desired state.
/// Complete snapshots and a monotonic version prevent CI updates erasing links
/// or an old publication overwriting a newer one.
pub(crate) async fn request(
    db: &crate::storage::StorageBackend,
    org: i64,
    session: SessionId,
    raw: &str,
) -> Result<i64, everruns_contracts::slack_action::SlackActionError> {
    use everruns_contracts::slack_action::SlackActionError as E;
    if raw.len() > 2_100_000 {
        return Err(E::InvalidArgument(
            "Code task publication is too large".into(),
        ));
    }
    let projection: Projection = serde_json::from_str(raw)
        .map_err(|_| E::InvalidArgument("Invalid code task publication".into()))?;
    projection.validate().map_err(E::InvalidArgument)?;
    let payload =
        serde_json::to_value(&projection).map_err(|e| E::InvalidArgument(e.to_string()))?;
    let changed: Option<i64> = sqlx::query_scalar("UPDATE slack_code_tasks SET desired=$3, version=version+1,next_attempt=GREATEST(next_attempt,NOW()) WHERE org_id=$1 AND session_id=$2 AND version=$4 AND NOT stopped AND NOT disabled AND NOT archived AND NOT EXISTS (SELECT 1 FROM slack_code_stops s WHERE s.endpoint_id=slack_code_tasks.endpoint_id AND s.team_id=slack_code_tasks.team_id AND s.slack_channel=slack_code_tasks.slack_channel) RETURNING version")
        .bind(org).bind(session.uuid()).bind(&payload).bind(projection.expected_revision).fetch_optional(db.pool()).await
        .map_err(|e| E::Transient(e.to_string()))?;
    if let Some(revision) = changed {
        return Ok(revision);
    }
    let current: Option<(i64,Value)> = sqlx::query_as("SELECT version,desired FROM slack_code_tasks WHERE org_id=$1 AND session_id=$2 AND NOT stopped AND NOT disabled AND NOT archived AND NOT EXISTS (SELECT 1 FROM slack_code_stops s WHERE s.endpoint_id=slack_code_tasks.endpoint_id AND s.team_id=slack_code_tasks.team_id AND s.slack_channel=slack_code_tasks.slack_channel)")
        .bind(org).bind(session.uuid()).fetch_optional(db.pool()).await.map_err(|e| E::Transient(e.to_string()))?;
    match current {
        Some((revision, desired))
            if desired == payload && revision == projection.expected_revision + 1 =>
        {
            Ok(revision)
        }
        Some((revision, _)) => Err(E::InvalidArgument(format!(
            "Publication revision changed to {revision}; resubmit a complete snapshot using this expected_revision"
        ))),
        None => Err(E::ChannelUnavailable),
    }
}

pub(super) async fn stopped_at(
    state: &SlackState,
    task: &Task,
) -> anyhow::Result<Option<chrono::DateTime<chrono::Utc>>> {
    let Some(channel) = task.slack_channel.as_deref() else {
        return Ok(None);
    };
    Ok(sqlx::query_scalar("SELECT stopped_at FROM slack_code_stops WHERE endpoint_id=$1 AND team_id=$2 AND slack_channel=$3 ORDER BY stopped_at DESC LIMIT 1")
        .bind(task.endpoint_id).bind(&task.team_id).bind(channel).fetch_optional(state.db.pool()).await?)
}
