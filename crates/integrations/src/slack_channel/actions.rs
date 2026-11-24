//! Slack vendor operations shared by Server and application-owned Framework bots.
use super::markdown::neutralize_broadcasts;
use super::web_api::SlackApiError;
use everruns_contracts::runtime::channel::DeliveryContext;
use everruns_contracts::slack_action::{SlackActionError, SlackActionOutcome, SlackFile};
use serde_json::{Value, json};
// THREAT[TM-DOS-031]: validate at the vendor seam as well as in model-facing tool arguments.
const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
pub fn action_error(error: SlackApiError) -> SlackActionError {
    match error {
        SlackApiError::RateLimited { retry_after } => SlackActionError::RateLimited {
            retry_after_secs: retry_after.map(|d| d.as_secs()),
        },
        SlackApiError::Permanent(message) => SlackActionError::Rejected(message),
        SlackApiError::Transient(message) => SlackActionError::Transient(message),
        SlackApiError::Delivery(
            everruns_contracts::runtime::channel::DeliveryResult::Partial { confirmed, cause },
        ) => SlackActionError::PartialDelivery {
            confirmed,
            retry_after_secs: cause.retry_after().map(|duration| duration.as_secs()),
            reason: match cause.as_ref() {
                everruns_contracts::runtime::channel::DeliveryResult::PermanentError(message)
                | everruns_contracts::runtime::channel::DeliveryResult::TransientError(message) => {
                    message.clone()
                }
                other => other.to_string(),
            },
        },
        SlackApiError::Delivery(outcome) => SlackActionError::DeliveryUnknown {
            confirmed: outcome.confirmed().to_vec(),
        },
    }
}

async fn client_scoped(
    api: &str,
    context: &DeliveryContext,
) -> Result<std::sync::Arc<channelkit_slack::Slack>, SlackApiError> {
    super::bridge::direct_factory()
        .map_err(super::web_api::provider_error)?
        .for_context(context, api)
        .await
        .map_err(super::web_api::provider_error)
}
async fn call_scoped(
    api: &str,
    context: &DeliveryContext,
    method: &str,
    payload: Value,
    side_effect: bool,
) -> Result<Value, SlackApiError> {
    client_scoped(api, context)
        .await?
        .api_call(method, payload, side_effect)
        .await
        .map_err(super::web_api::provider_error)
}

pub async fn add_reaction(
    api: &str,
    token: &str,
    channel: &str,
    ts: &str,
    name: &str,
) -> Result<SlackActionOutcome, SlackApiError> {
    add_reaction_scoped(
        api,
        &super::web_api::tool_context(token, channel, ""),
        ts,
        name,
    )
    .await
}
pub async fn update_message(
    api: &str,
    token: &str,
    channel: &str,
    ts: &str,
    text: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    update_message_scoped(
        api,
        &super::web_api::tool_context(token, channel, ""),
        ts,
        text,
    )
    .await
}
pub async fn lookup_user(
    api: &str,
    token: &str,
    user: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    lookup_user_scoped(api, &super::web_api::tool_context(token, "", ""), user).await
}
pub async fn upload_file(
    api: &str,
    token: &str,
    channel: &str,
    thread: Option<&str>,
    filename: &str,
    content: Vec<u8>,
    comment: Option<&str>,
) -> Result<SlackActionOutcome, SlackActionError> {
    upload_file_scoped(
        api,
        &super::web_api::tool_context(token, channel, thread.unwrap_or_default()),
        filename,
        content,
        comment,
    )
    .await
}
pub async fn share_message_files(
    api: &str,
    token: &str,
    channel: &str,
    thread: Option<&str>,
    files: Vec<SlackFile>,
) -> Result<(), SlackActionError> {
    share_message_files_scoped(
        api,
        &super::web_api::tool_context(token, channel, thread.unwrap_or_default()),
        files,
    )
    .await
}

pub async fn add_reaction_scoped(
    api_base: &str,
    context: &DeliveryContext,
    timestamp: &str,
    name: &str,
) -> Result<SlackActionOutcome, SlackApiError> {
    let payload = json!({ "channel": context.channel_id, "timestamp": timestamp, "name": name });
    match call_scoped(api_base, context, "reactions.add", payload, false).await {
        Ok(_) => Ok(SlackActionOutcome::ReactionAdded {
            already_reacted: false,
        }),
        // The agent's intent — that emoji is on that message — is satisfied
        // either way, so a duplicate is a success that says so rather than an
        // error the model has to interpret.
        Err(error) if error.code() == Some("already_reacted") => {
            Ok(SlackActionOutcome::ReactionAdded {
                already_reacted: true,
            })
        }
        Err(error) => Err(error),
    }
}

pub async fn update_message_scoped(
    api_base: &str,
    context: &DeliveryContext,
    timestamp: &str,
    text: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    use channelkit::Adapter;
    let client = client_scoped(api_base, context)
        .await
        .map_err(action_error)?;
    let reference = super::bridge::reference(&client, context, timestamp)
        .map_err(super::web_api::provider_error)
        .map_err(action_error)?;
    client
        .edit(
            &reference,
            channelkit::Content::Markdown(neutralize_broadcasts(text).into_owned()),
        )
        .await
        .map_err(super::web_api::provider_error)
        .map_err(action_error)?;
    Ok(SlackActionOutcome::MessageUpdated {
        channel: context.channel_id.clone(),
        timestamp: timestamp.to_string(),
    })
}

pub async fn lookup_user_scoped(
    api_base: &str,
    context: &DeliveryContext,
    user_id: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    let body = call_scoped(
        api_base,
        context,
        "users.info",
        json!({ "user": user_id }),
        false,
    )
    .await
    .map_err(action_error)?;
    let user = body.get("user").ok_or_else(|| {
        SlackActionError::Transient("Slack users.info returned no user".to_string())
    })?;
    let profile = user.get("profile");

    // Only the fields an agent needs to address someone. `users.info` also
    // carries email, phone, and title; forwarding those would widen what the
    // model sees for no gain.
    Ok(SlackActionOutcome::User {
        user_id: user_id.to_string(),
        display_name: profile
            .and_then(|p| p.get("display_name"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        real_name: profile
            .and_then(|p| p.get("real_name"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        is_bot: user.get("is_bot").and_then(Value::as_bool).unwrap_or(false),
        tz: user
            .get("tz")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

/// Share a file, via the three-step external upload flow.
///
/// `files.upload` is retired, so this is `files.getUploadURLExternal` for a
/// one-time URL, a raw PUT/POST of the bytes to it, then
/// `files.completeUploadExternal` to place the file in the conversation. Only
/// the first and last speak the `ok`/`error` envelope; the middle step is a
/// plain HTTP upload whose failures are classified here.
pub async fn upload_file_scoped(
    api_base: &str,
    context: &DeliveryContext,
    filename: &str,
    content: Vec<u8>,
    initial_comment: Option<&str>,
) -> Result<SlackActionOutcome, SlackActionError> {
    if content.is_empty() || content.len() > MAX_UPLOAD_BYTES {
        return Err(SlackActionError::InvalidArgument(
            "Slack file must contain 1 byte to 8 MiB".into(),
        ));
    }
    let client = client_scoped(api_base, context)
        .await
        .map_err(action_error)?;
    let target = super::bridge::target(&client, context)
        .map_err(super::web_api::provider_error)
        .map_err(action_error)?;
    let file = client
        .upload_with_options(
            &target,
            filename,
            &content,
            &channelkit_slack::SlackUploadOptions {
                markdown_comment: initial_comment
                    .map(|text| neutralize_broadcasts(text).into_owned()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            let error = super::web_api::provider_error(error);
            if error.code() == Some("missing_scope") {
                SlackActionError::Rejected(
                    "missing_scope: reconnect the Slack app and approve files:write for file uploads".into(),
                )
            } else {
                action_error(error)
            }
        })?;
    Ok(SlackActionOutcome::FileUploaded {
        file_id: file.id,
        permalink: file.permalink,
    })
}

pub async fn share_message_files_scoped(
    api_base: &str,
    context: &DeliveryContext,
    files: Vec<SlackFile>,
) -> Result<(), SlackActionError> {
    let mut confirmed = Vec::new();
    for file in files {
        match upload_file_scoped(api_base, context, &file.filename, file.content, None).await {
            Ok(SlackActionOutcome::FileUploaded { file_id, .. }) => {
                confirmed.push(format!("file:{file_id}"))
            }
            Ok(_) => unreachable!("upload returns a file receipt"),
            Err(error) => return Err(with_confirmed(error, confirmed)),
        }
    }
    Ok(())
}

/// Preserve an accepted prefix when an explicit post subsequently shares files.
pub fn with_confirmed(error: SlackActionError, mut confirmed: Vec<String>) -> SlackActionError {
    match error {
        SlackActionError::DeliveryUnknown { confirmed: suffix } => {
            confirmed.extend(suffix);
            SlackActionError::DeliveryUnknown { confirmed }
        }
        SlackActionError::PartialDelivery {
            confirmed: suffix,
            retry_after_secs,
            reason,
        } => {
            confirmed.extend(suffix);
            SlackActionError::PartialDelivery {
                confirmed,
                retry_after_secs,
                reason,
            }
        }
        error if confirmed.is_empty() => error,
        error => SlackActionError::PartialDelivery {
            confirmed,
            reason: error.to_string(),
            retry_after_secs: match error {
                SlackActionError::RateLimited { retry_after_secs } => retry_after_secs,
                _ => None,
            },
        },
    }
}
