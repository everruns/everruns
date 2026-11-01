use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Projection {
    pub expected_revision: i64,
    pub context: Vec<ContextItem>,
    pub diff: Option<String>,
    pub base_branch: String,
    pub head_branch: String,
    pub summary: String,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextItem {
    key: String,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

impl Projection {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.context.len() > 4
            || self.summary.chars().count() > 2800
            || self.expected_revision < 0
            || self.expected_revision == i64::MAX
        {
            return Err("Use at most four context items, a 2800-character summary, and a nonnegative revision".into());
        }
        let mut keys = std::collections::HashSet::new();
        for item in &self.context {
            if item.key.is_empty()
                || item.key.len() > 64
                || item.key == "everruns"
                || !keys.insert(&item.key)
                || item.label.is_empty()
                || item.label.chars().count() > 128
            {
                return Err("Invalid or duplicate context key/label".into());
            }
            if let Some(url) = &item.url {
                let parsed = url::Url::parse(url).map_err(|_| "Invalid context URL")?;
                if parsed.scheme() != "https"
                    || parsed.host_str().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                    || url.len() > 2048
                {
                    return Err("Context links must be HTTPS URLs without credentials".into());
                }
            }
        }
        if self.diff.as_ref().is_some_and(|diff| {
            diff.len() > 1_000_000
                || (!diff.is_empty()
                    && !diff.starts_with("diff --git ")
                    && !diff.starts_with("--- "))
        }) {
            return Err("Diff must be a unified diff of at most 1,000,000 bytes; publish a review link for larger changes".into());
        }
        if self.base_branch.len() > 255 || self.head_branch.len() > 255 {
            return Err("Branch name too long".into());
        }
        if self.complete && self.summary.trim().is_empty() {
            return Err("Completion requires a final summary".into());
        }
        Ok(())
    }
}

pub(super) async fn step(state: &SlackState) -> anyhow::Result<bool> {
    let mut tx = everruns_durable::begin_transaction(state.db.pool()).await?;
    // Keep the row lock over one bounded external step. A persisted stop or
    // revocation orders after any already-in-flight call, before every later one.
    // THREAT[TM-SLACK-015]: independent durable stop markers suppress restart delivery.
    let Some(task)=sqlx::query_as::<_,Task>("SELECT * FROM slack_code_tasks WHERE NOT disabled AND NOT archived AND NOT stopped AND session_id IS NOT NULL AND slack_channel IS NOT NULL AND NOT EXISTS (SELECT 1 FROM slack_code_stops s WHERE s.endpoint_id=slack_code_tasks.endpoint_id AND s.team_id=slack_code_tasks.team_id AND s.slack_channel=slack_code_tasks.slack_channel) AND next_attempt<=NOW() ORDER BY next_attempt,id LIMIT 1 FOR UPDATE SKIP LOCKED")
        .fetch_optional(&mut *tx).await? else {return Ok(false)};
    let result = project(state, &task, &mut tx).await;
    if let Err(error) = result {
        let slack = error.downcast_ref::<SlackApiError>();
        let disabled = slack.is_some_and(|e| matches!(e, SlackApiError::Permanent(_)));
        let wait = match slack {
            Some(SlackApiError::RateLimited {
                retry_after: Some(d),
            }) => d.as_secs().clamp(1, 3600) as i64,
            _ => 30,
        };
        sqlx::query("UPDATE slack_code_tasks SET disabled=$2, stopped=stopped OR $2, next_attempt=NOW()+make_interval(secs=>$3) WHERE id=$1")
            .bind(task.id).bind(disabled).bind(wait as f64).execute(&mut *tx).await?;
        if disabled && let Some(session) = task.session_id {
            worker::cancel(state, session).await?;
        }
        tracing::warn!(task=%task.id,%error,"Slack Code projection deferred");
    }
    everruns_durable::commit_transaction(tx).await?;
    Ok(true)
}

async fn project(
    state: &SlackState,
    task: &Task,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> anyhow::Result<()> {
    let (_, config) = match endpoint(state, task.endpoint_id).await {
        Ok(value) => value,
        Err(error) if error.is::<EndpointUnavailable>() => {
            sqlx::query("UPDATE slack_code_tasks SET disabled=TRUE,stopped=TRUE WHERE id=$1")
                .bind(task.id)
                .execute(&mut **tx)
                .await?;
            if let Some(session) = task.session_id {
                worker::cancel(state, session).await?;
            }
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let channel = task
        .slack_channel
        .as_deref()
        .context("Missing code channel")?;
    if config.team_id.as_deref() != Some(&task.team_id) {
        bail!("Workspace binding changed")
    }
    if task.installation != installation(&config) {
        let info = channel_info(&state.code_api_base, &config, &task.team_id, channel).await?;
        if !is_code(&info) {
            bail!("Bound conversation is no longer a code channel")
        }
        sqlx::query("UPDATE slack_code_tasks SET installation=$2 WHERE id=$1")
            .bind(task.id)
            .bind(installation(&config))
            .execute(&mut **tx)
            .await?;
    }
    let session = SessionId::from_uuid(task.session_id.context("Missing session")?);
    let events = state
        .db
        .list_events_advanced(&crate::storage::ListEventsParams {
            session_id: session,
            after_sequence: Some(task.cursor.try_into()?),
            limit: Some(1),
            ..Default::default()
        })
        .await?;
    if let Some(event) = events.first() {
        match event.event_type.as_str() {
            "turn.started" | "tool.started" => {
                status(state, &config, task, tx, "processing").await?;
                sqlx::query("UPDATE slack_code_tasks SET ready=FALSE WHERE id=$1")
                    .bind(task.id)
                    .execute(&mut **tx)
                    .await?;
            }
            "turn.completed" | "turn.failed" | "turn.cancelled" | "turn.sealed" => {
                status(state, &config, task, tx, "active").await?;
                sqlx::query("UPDATE slack_code_tasks SET ready=$2 WHERE id=$1")
                    .bind(task.id)
                    .bind(matches!(
                        event.event_type.as_str(),
                        "turn.completed" | "turn.sealed"
                    ))
                    .execute(&mut **tx)
                    .await?;
            }
            "output.message.completed" => {
                if event.data["message"]["phase_source"] != "communication"
                    && event.data["message"]["phase"] != "commentary"
                    && let Some(text) = everruns_core::channel_runtime::response_text(&event.data)
                {
                    post(
                        state,
                        &config,
                        task,
                        &format!("event:{}", event.sequence),
                        &text,
                        None,
                    )
                    .await?;
                }
            }
            "tool.call_requested" => {
                let pending = crate::api::tool_approvals::pending_approvals_from_events(
                    std::slice::from_ref(event),
                );
                for approval in pending {
                    let binding = json!({"task":task.id,"request":approval.request_call_id,"fingerprint":approval.request.fingerprint});
                    let value = serde_json::to_string(&binding)?;
                    if value.len() > 1900 {
                        bail!("Approval binding exceeds Slack action limit")
                    }
                    let preview = serde_json::to_string_pretty(&approval.request.arguments)?;
                    let preview: String = preview.chars().take(2000).collect();
                    let blocks = json!([
                        {"type":"section","text":{"type":"mrkdwn","text":format!("Approval required: *{}*\n```{}```",approval.request.tool,preview.replace('`',"'"))}},
                        {"type":"actions","elements":[
                            {"type":"button","text":{"type":"plain_text","text":"Approve once"},"action_id":"everruns_code_allow","value":value},
                            {"type":"button","text":{"type":"plain_text","text":"Reject"},"action_id":"everruns_code_reject","value":value}
                        ]}
                    ]);
                    post(
                        state,
                        &config,
                        task,
                        &approval.request_call_id,
                        "Tool approval required",
                        Some(blocks),
                    )
                    .await?;
                }
                status(state, &config, task, tx, "suspended").await?;
            }
            "session.title.updated" => {
                if let Some(title) = event.data["title"].as_str() {
                    call(
                        state,
                        &config,
                        task,
                        "agents.sessions.rename",
                        json!({"channel_id":channel,"title":title}),
                    )
                    .await?;
                }
            }
            _ => {}
        }
        sqlx::query("UPDATE slack_code_tasks SET cursor=$2,next_attempt=NOW() WHERE id=$1")
            .bind(task.id)
            .bind(event.sequence)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    }
    if task.version != task.published {
        let desired: Projection = serde_json::from_value(task.desired.clone())?;
        desired.validate().map_err(anyhow::Error::msg)?;
        let previous: Projection = serde_json::from_value(task.published_state.clone())?;
        let mut items = serde_json::to_value(&desired.context)?
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(link) = state.session_link(session) {
            items.push(json!({"key":"everruns","label":"Everruns session","url":link}));
        }
        call(
            state,
            &config,
            task,
            "agents.conversations.setProperties",
            json!({"channel_id":channel,"code_channel":{"context_bar_items":items}}),
        )
        .await?;
        if let Some(diff) = &desired.diff {
            if desired.diff != previous.diff
                || desired.base_branch != previous.base_branch
                || desired.head_branch != previous.head_branch
            {
                // Slack requires nonempty content even for an empty git diff.
                let content = if diff.is_empty() { "\n" } else { diff };
                let response=view(state,&config,task,json!({"channel_id":channel,"type":"diff","content":content,"base_branch":desired.base_branch,"head_branch":desired.head_branch})).await?;
                if let Some(view) = response["view_id"].as_str() {
                    sqlx::query("UPDATE slack_code_tasks SET diff_view_id=$2,diff_file_id=$3,diff_content_version=$4 WHERE id=$1")
                        .bind(task.id)
                        .bind(view)
                        .bind(response["file_id"].as_str())
                        .bind(response["content_version"].as_i64())
                        .execute(&mut **tx)
                        .await?;
                }
            }
        } else if let Some(view) = &task.diff_view_id {
            if view == "code" {
                // The built-in Code tab cannot be removed. Clear its content
                // only while Slack still exposes our exact published receipt;
                // a newer snapshot, including another bot's, must survive.
                let info = call(
                    state,
                    &config,
                    task,
                    "conversations.info",
                    json!({"channel":channel}),
                )
                .await?;
                let current = &info["channel"]["properties"]["code_channel"];
                if task.diff_file_id.is_some()
                    && task.diff_content_version.is_some()
                    && current["diff_file_id"].as_str() == task.diff_file_id.as_deref()
                    && current["diff_file_version"].as_i64() == task.diff_content_version
                {
                    call(state, &config, task, "agents.conversations.setView", json!({"channel_id":channel,"type":"diff","content":"\n","base_branch":"","head_branch":""})).await?;
                }
            } else {
                // Encoded tabs can be removed by their own persisted identity.
                call(
                    state,
                    &config,
                    task,
                    "agents.conversations.removeView",
                    json!({"channel_id":channel,"view_id":view}),
                )
                .await?;
            }
            sqlx::query("UPDATE slack_code_tasks SET diff_view_id=NULL,diff_file_id=NULL,diff_content_version=NULL WHERE id=$1")
                .bind(task.id)
                .execute(&mut **tx)
                .await?;
        }
        if !desired.summary.is_empty() {
            view(state,&config,task,json!({"channel_id":channel,"type":"block_kit","view_key":"everruns-summary","name":"Summary","blocks":[{"type":"section","text":{"type":"mrkdwn","text":desired.summary}}]})).await?;
        }
        sqlx::query("UPDATE slack_code_tasks SET published=$2,published_state=$3 WHERE id=$1")
            .bind(task.id)
            .bind(task.version)
            .bind(&task.desired)
            .execute(&mut **tx)
            .await?;
    }
    let desired: Projection = serde_json::from_value(task.desired.clone())?;
    if desired.complete && task.ready {
        let summary_ts = match &task.summary_ts {
            Some(ts) => ts.clone(),
            None => {
                let ts = post(
                    state,
                    &config,
                    task,
                    &format!("final-summary:{}", task.version),
                    &desired.summary,
                    None,
                )
                .await?;
                sqlx::query("UPDATE slack_code_tasks SET summary_ts=$2 WHERE id=$1")
                    .bind(task.id)
                    .bind(&ts)
                    .execute(&mut **tx)
                    .await?;
                ts
            }
        };
        status(state, &config, task, tx, "closed").await?;
        call(
            state,
            &config,
            task,
            "agents.conversations.archive",
            json!({"channel_id":channel,"summary_message_ts":summary_ts}),
        )
        .await?;
        sqlx::query("UPDATE slack_code_tasks SET archived=TRUE WHERE id=$1")
            .bind(task.id)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query("UPDATE slack_code_tasks SET next_attempt=NOW()+INTERVAL '2 seconds' WHERE id=$1")
        .bind(task.id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn call(
    state: &SlackState,
    config: &SlackChannelConfig,
    task: &Task,
    method: &str,
    payload: Value,
) -> anyhow::Result<Value> {
    if store::stopped_at(state, task).await?.is_some() {
        bail!("Slack Code task stopped")
    }
    match slack_api_call(&state.code_api_base, &config.bot_token, method, payload).await {
        Ok(value) => Ok(value),
        Err(error)
            if method == "agents.conversations.archive"
                && error.code() == Some("already_archived") =>
        {
            Ok(json!({"ok":true}))
        }
        Err(error) => Err(error.into()),
    }
}

async fn post(
    state: &SlackState,
    config: &SlackChannelConfig,
    task: &Task,
    key: &str,
    text: &str,
    blocks: Option<Value>,
) -> anyhow::Result<String> {
    let channel = task
        .slack_channel
        .as_deref()
        .context("Missing destination")?;
    let mut last = String::new();
    let mut payloads = super::super::delivery::build_post_payloads(channel, "", text, None);
    if let Some(blocks) = blocks {
        payloads = vec![json!({"channel":channel,"text":text,"blocks":blocks})];
    }
    for (index, mut payload) in payloads.into_iter().enumerate() {
        payload["client_msg_id"] = json!(
            stable_id(
                task.endpoint_id,
                &task.team_id,
                channel,
                &format!("{key}:{index}")
            )
            .to_string()
        );
        let response = call(state, config, task, "chat.postMessage", payload).await?;
        last = response["ts"]
            .as_str()
            .context("Post returned no timestamp")?
            .to_string();
    }
    Ok(last)
}

async fn status(
    state: &SlackState,
    config: &SlackChannelConfig,
    task: &Task,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    value: &str,
) -> anyhow::Result<()> {
    if task.lifecycle != value {
        call(
            state,
            config,
            task,
            "agents.sessions.setStatus",
            json!({"channel_id":task.slack_channel,"status":value}),
        )
        .await?;
        sqlx::query("UPDATE slack_code_tasks SET lifecycle=$2 WHERE id=$1")
            .bind(task.id)
            .bind(value)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn view(
    state: &SlackState,
    config: &SlackChannelConfig,
    task: &Task,
    payload: Value,
) -> anyhow::Result<Value> {
    let kind = payload["type"].as_str().unwrap_or("view").to_owned();
    match call(state, config, task, "agents.conversations.setView", payload).await {
        Ok(value) => Ok(value),
        Err(error)
            if error
                .downcast_ref::<SlackApiError>()
                .is_some_and(|e| matches!(e.code(), Some("too_many_views" | "not_view_owner"))) =>
        {
            post(state,config,task,&format!("view-fallback:{}:{kind}",task.version),"Slack could not add this review view. Use the repository or PR links in the context bar to review the changes.",None).await?;
            Ok(json!({"ok":true,"fallback":true}))
        }
        Err(error) => Err(error),
    }
}
