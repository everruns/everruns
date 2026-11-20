//! Slack vendor operations shared by Server and application-owned Framework bots.
use super::markdown::{markdown_block, neutralize_broadcasts};
use super::web_api::{SlackApiError, parse_retry_after, slack_api_call};
use everruns_contracts::slack_action::{SlackActionError, SlackActionOutcome, SlackFile};
use serde_json::{Value, json};
use tracing::warn;
// THREAT[TM-DOS-031]: validate at the vendor seam as well as in model-facing tool arguments.
const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
pub fn action_error(error: SlackApiError) -> SlackActionError {
    match error {
        SlackApiError::RateLimited { retry_after } => SlackActionError::RateLimited {
            retry_after_secs: retry_after.map(|d| d.as_secs()),
        },
        SlackApiError::Permanent(message) => SlackActionError::Rejected(message),
        SlackApiError::Transient(message) => SlackActionError::Transient(message),
    }
}

pub async fn add_reaction(
    api_base: &str,
    bot_token: &str,
    channel: &str,
    timestamp: &str,
    name: &str,
) -> Result<SlackActionOutcome, SlackApiError> {
    let payload = json!({ "channel": channel, "timestamp": timestamp, "name": name });
    match slack_api_call(api_base, bot_token, "reactions.add", payload).await {
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

pub async fn update_message(
    api_base: &str,
    bot_token: &str,
    channel: &str,
    timestamp: &str,
    text: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    // Mirrors the delivery path's rendering choice: `markdown` blocks, so agent
    // output reaches Slack as the Markdown it actually is rather than being
    // reinterpreted as the much smaller `mrkdwn` dialect. `text` is still sent
    // as the notification/fallback string.
    let payload = json!({
        "channel": channel,
        "ts": timestamp,
        "text": neutralize_broadcasts(text),
        "blocks": [markdown_block(text)],
    });
    slack_api_call(api_base, bot_token, "chat.update", payload)
        .await
        .map_err(action_error)?;
    Ok(SlackActionOutcome::MessageUpdated {
        channel: channel.to_string(),
        timestamp: timestamp.to_string(),
    })
}

pub async fn lookup_user(
    api_base: &str,
    bot_token: &str,
    user_id: &str,
) -> Result<SlackActionOutcome, SlackActionError> {
    let body = slack_api_call(
        api_base,
        bot_token,
        "users.info",
        json!({ "user": user_id }),
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
pub async fn upload_file(
    api_base: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: Option<&str>,
    filename: &str,
    content: Vec<u8>,
    initial_comment: Option<&str>,
) -> Result<SlackActionOutcome, SlackActionError> {
    if content.is_empty() {
        return Err(SlackActionError::InvalidArgument(
            "file content must not be empty".to_string(),
        ));
    }
    if content.len() > MAX_UPLOAD_BYTES {
        return Err(SlackActionError::InvalidArgument(format!(
            "file content is {} bytes, over the {MAX_UPLOAD_BYTES} byte limit",
            content.len()
        )));
    }

    let reserve = slack_api_call(
        api_base,
        bot_token,
        "files.getUploadURLExternal",
        json!({ "filename": filename, "length": content.len() }),
    )
    .await
    .map_err(action_error)?;

    let upload_url = reserve
        .get("upload_url")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SlackActionError::Transient(
                "Slack files.getUploadURLExternal returned no upload_url".to_string(),
            )
        })?;
    let file_id = reserve
        .get("file_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SlackActionError::Transient(
                "Slack files.getUploadURLExternal returned no file_id".to_string(),
            )
        })?
        .to_string();

    // THREAT[TM-FW-003]: the reservation is remote input. Never follow it to an arbitrary origin,
    // redirect bytes to another host, or expose the signed URL in an error.
    let upload = url::Url::parse(upload_url)
        .map_err(|_| SlackActionError::Rejected("Invalid Slack upload URL".into()))?;
    let api = url::Url::parse(api_base)
        .map_err(|_| SlackActionError::Rejected("Invalid Slack API root".into()))?;
    let slack_host = upload
        .host_str()
        .is_some_and(|host| host == "files.slack.com" || host.ends_with(".slack.com"));
    let trusted_test_origin =
        api_base != super::web_api::SLACK_API_BASE && upload.origin() == api.origin();
    if !upload.username().is_empty()
        || upload.password().is_some()
        || !((upload.scheme() == "https" && slack_host) || trusted_test_origin)
    {
        return Err(SlackActionError::Rejected(
            "Slack upload URL has an untrusted origin".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| SlackActionError::Transient("Could not create Slack upload client".into()))?;
    let response = client
        .post(upload_url)
        .body(content)
        .send()
        .await
        .map_err(|_| SlackActionError::Transient("Slack upload transport failed".into()))?;
    if !response.status().is_success() {
        let status = response.status();
        let retry_after = parse_retry_after(response.headers());
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SlackActionError::RateLimited {
                retry_after_secs: retry_after.map(|d| d.as_secs()),
            });
        }
        // 4xx on the one-time URL means the reservation no longer applies;
        // retrying the same URL cannot help, so it is reported as a rejection
        // rather than something to back off on.
        return Err(if status.is_client_error() {
            SlackActionError::Rejected(format!("Slack rejected the file upload ({status})"))
        } else {
            SlackActionError::Transient(format!("Slack file upload failed ({status})"))
        });
    }

    let mut file_entry = json!({ "id": file_id });
    if let Some(comment) = initial_comment {
        file_entry["title"] = json!(filename);
        // A `markdown` block: `initial_comment` would be read as `mrkdwn`.
        let mut payload = json!({
            "files": [file_entry],
            "channel_id": channel,
            "blocks": [markdown_block(comment)],
        });
        if let Some(thread_ts) = thread_ts {
            payload["thread_ts"] = json!(thread_ts);
        }
        return complete_upload(api_base, bot_token, payload, file_id).await;
    }

    let mut payload = json!({ "files": [file_entry], "channel_id": channel });
    if let Some(thread_ts) = thread_ts {
        payload["thread_ts"] = json!(thread_ts);
    }
    complete_upload(api_base, bot_token, payload, file_id).await
}

/// Share the files of a message whose text Slack already accepted, into the
/// same conversation, in order.
///
/// The text is posted first because it carries the message reference later
/// edits need, and reads naturally above what it introduces. A failure here
/// is therefore partial delivery, reported as a rejection that says so: a
/// blind retry of the whole message would post the text twice.
pub async fn share_message_files(
    api_base: &str,
    bot_token: &str,
    channel: &str,
    thread_ts: Option<&str>,
    files: Vec<SlackFile>,
) -> Result<(), SlackActionError> {
    let total = files.len();
    for (index, file) in files.into_iter().enumerate() {
        let filename = file.filename.clone();
        if let Err(error) = upload_file(
            api_base,
            bot_token,
            channel,
            thread_ts,
            &file.filename,
            file.content,
            None,
        )
        .await
        {
            let reason = match &error {
                SlackActionError::Rejected(message) if message.contains("missing_scope") => {
                    "the Slack app lacks files:write; reconnect it to approve file uploads"
                        .to_string()
                }
                other => other.to_string(),
            };
            return Err(SlackActionError::Rejected(format!(
                "The message text was posted, and {index} of {total} files, but {filename} was not: {reason}. Do not resend the text."
            )));
        }
    }
    Ok(())
}

async fn complete_upload(
    api_base: &str,
    bot_token: &str,
    payload: Value,
    file_id: String,
) -> Result<SlackActionOutcome, SlackActionError> {
    let body = slack_api_call(api_base, bot_token, "files.completeUploadExternal", payload)
        .await
        .map_err(action_error)?;
    let permalink = body
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .and_then(|file| file.get("permalink"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if permalink.is_none() {
        // Not fatal: the file is shared, we just cannot link to it.
        warn!(file_id = %file_id, "Slack completeUploadExternal returned no permalink");
    }
    Ok(SlackActionOutcome::FileUploaded { file_id, permalink })
}
