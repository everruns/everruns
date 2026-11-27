//! Explicitly opted-in provider contracts; no user-origin ingress is simulated here.
//! Run with `--features slack-live-tests --test slack_channel_live`.
//! Supply TEST_SLACK_BOT_TOKEN, TEST_SLACK_TEST_CHANNEL, TEST_SLACK_TEAM_ID and,
//! for the stream test, the opted-in TEST_SLACK_USER_ID. Missing inputs fail.
use anyhow::{Context, Result, ensure};
use channelkit::{Attachment, http::Transport};
use everruns_contracts::{
    runtime::channel::{ChannelStreamDelivery, DeliveryContext, DeliveryResult, DeliveryTarget},
    slack_action::{SlackActionError, SlackActionOutcome},
};
use everruns_integrations::slack_channel::{
    actions,
    bridge::{INSTALLATION_ID, SlackAdapterFactory, TEAM_ID},
    channelkit_surfaces::ChannelkitSurfaces,
    surfaces::{SLACK_RECIPIENT_TEAM_ID, SLACK_RECIPIENT_USER_ID},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

const API: &str = "https://slack.com/api";

fn required(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{name} is required for opted-in live tests"))
}

struct Fixture {
    factory: SlackAdapterFactory,
    context: DeliveryContext,
    messages: Vec<(String, String)>,
    files: Vec<String>,
    streams: Vec<String>,
    nonce: String,
}

impl Fixture {
    async fn new() -> Result<Self> {
        let mut context = DeliveryTarget::new(required("TEST_SLACK_TEST_CHANNEL")?, "")
            .context(required("TEST_SLACK_BOT_TOKEN")?);
        context
            .extra
            .insert(INSTALLATION_ID.into(), "live-contract".into());
        context
            .extra
            .insert(TEAM_ID.into(), required("TEST_SLACK_TEAM_ID")?);
        let fixture = Self {
            factory: SlackAdapterFactory::new(Arc::new(channelkit::http::ReqwestTransport::new()?)),
            context,
            messages: vec![],
            files: vec![],
            streams: vec![],
            nonce: format!("channelkit-live-{}", uuid::Uuid::new_v4()),
        };
        // Identity discovery checks the expected workspace before the first write.
        fixture.client().await?;
        Ok(fixture)
    }

    async fn client(&self) -> Result<Arc<channelkit_slack::Slack>> {
        Ok(self.factory.for_context(&self.context, API).await?)
    }

    async fn call(&self, method: &str, payload: Value, write: bool) -> Result<Value> {
        self.client()
            .await?
            .api_call(method, payload, write)
            .await
            .with_context(|| format!("Slack {method} failed"))
    }

    async fn root(&mut self) -> Result<()> {
        let posted = self
            .call(
                "chat.postMessage",
                json!({
                    "channel": self.context.channel_id, "text": self.nonce,
                }),
                true,
            )
            .await?;
        let ts = posted["ts"]
            .as_str()
            .context("root receipt missing")?
            .to_owned();
        let channel = posted["channel"]
            .as_str()
            .context("channel receipt missing")?;
        if self.context.channel_id.starts_with('U') {
            ensure!(
                channel.starts_with('D'),
                "test user's post did not return a DM"
            );
        } else {
            ensure!(
                channel == self.context.channel_id,
                "post changed its destination"
            );
        }
        self.context.channel_id = channel.to_owned();
        self.messages
            .push((self.context.channel_id.clone(), ts.clone()));
        self.context.thread_ref = ts;
        println!(
            "{}: owned thread {}:{}",
            self.nonce, self.context.channel_id, self.context.thread_ref
        );
        Ok(())
    }

    async fn reaction_readback(&self) -> Result<Value> {
        // Independent provider readback: this legacy read requires GET parameters.
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("channel", &self.context.channel_id)
            .append_pair("timestamp", &self.context.thread_ref)
            .finish();
        let response = channelkit::http::ReqwestTransport::new()?
            .execute(channelkit::http::Request {
                method: "GET".into(),
                url: format!("{API}/reactions.get?{query}"),
                headers: BTreeMap::from([(
                    "authorization".into(),
                    format!("Bearer {}", self.context.auth_token),
                )]),
                body: vec![],
                max_response_bytes: 1024 * 1024,
            })
            .await?;
        ensure!(response.status == 200, "reaction readback HTTP failure");
        let value: Value = serde_json::from_slice(&response.body)?;
        ensure!(value["ok"] == true, "reaction readback was refused");
        Ok(value)
    }

    async fn replies(&mut self) -> Result<Vec<Value>> {
        let response = self
            .call(
                "conversations.replies",
                json!({
                    "channel": self.context.channel_id, "ts": self.context.thread_ref, "limit": 100,
                }),
                false,
            )
            .await?;
        ensure!(
            response["has_more"] != true,
            "unexpected fixture pagination"
        );
        let messages = response["messages"]
            .as_array()
            .context("messages missing")?
            .clone();
        // Only remember messages authored by this bot in the new fixture thread.
        let identity = self.call("auth.test", json!({}), false).await?;
        for message in &messages {
            if message["user"] == identity["user_id"]
                && let Some(ts) = message["ts"].as_str()
            {
                let reference = (self.context.channel_id.clone(), ts.to_owned());
                if !self.messages.contains(&reference) {
                    self.messages.push(reference);
                }
            }
        }
        Ok(messages)
    }

    fn surfaces(&self) -> ChannelkitSurfaces<'_> {
        ChannelkitSurfaces {
            api_base: API,
            factory: &self.factory,
        }
    }

    async fn cleanup(&self) -> Result<()> {
        let mut failed = false;
        for stream in &self.streams {
            failed |= self.surfaces().stop(stream, &self.context).await != DeliveryResult::Ok;
        }
        for file in &self.files {
            failed |= cleanup_failed(self.call("files.delete", json!({"file":file}), true).await);
        }
        for (channel, ts) in self.messages.iter().rev() {
            failed |= cleanup_failed(
                self.call("chat.delete", json!({"channel":channel,"ts":ts}), true)
                    .await,
            );
        }
        ensure!(!failed, "cleanup incomplete for {}", self.nonce);
        Ok(())
    }
}

fn cleanup_failed(result: Result<Value>) -> bool {
    match result {
        Ok(_) => false,
        // Deleting a native file also removes its sharing message. Both handles
        // belong to this fixture; an explicit already-absent response is settled.
        Err(error)
            if matches!(error.downcast_ref::<channelkit::Error>(),
            Some(channelkit::Error::Rejected { code }) if matches!(code.as_str(), "message_not_found" | "file_not_found" | "file_deleted")) =>
        {
            false
        }
        Err(error) => {
            eprintln!("owned cleanup: {error:#}");
            true
        }
    }
}

#[tokio::test]
async fn shared_files_and_reactions_have_provider_readback() -> Result<()> {
    let mut fixture = Fixture::new().await?;
    let result = async {
        fixture.root().await?;
        actions::add_reaction_scoped(API, &fixture.context, &fixture.context.thread_ref, "eyes")
            .await?;
        let duplicate = actions::add_reaction_scoped(
            API,
            &fixture.context,
            &fixture.context.thread_ref,
            "eyes",
        )
        .await?;
        ensure!(
            matches!(
                duplicate,
                SlackActionOutcome::ReactionAdded {
                    already_reacted: true
                }
            ),
            "reaction was not idempotent"
        );
        let bytes = fixture.nonce.as_bytes().to_vec();
        let uploaded = actions::upload_file_scoped(
            API,
            &fixture.context,
            "channelkit-contract.txt",
            bytes.clone(),
            None,
        )
        .await?;
        let SlackActionOutcome::FileUploaded { file_id, .. } = uploaded else {
            anyhow::bail!("file receipt missing");
        };
        fixture.files.push(file_id.clone());
        let attachment = Attachment {
            id: file_id,
            name: None,
            media_type: None,
            size: None,
        };
        ensure!(
            fixture
                .client()
                .await?
                .download(&attachment, 25 * 1024 * 1024)
                .await?
                == bytes,
            "file bytes differ"
        );
        ensure!(
            fixture
                .client()
                .await?
                .download(&attachment, 1)
                .await
                .is_err(),
            "download ignored its byte ceiling"
        );
        ensure!(
            matches!(
                actions::upload_file_scoped(API, &fixture.context, "empty.txt", vec![], None).await,
                Err(SlackActionError::InvalidArgument(_))
            ),
            "empty upload was not refused"
        );
        ensure!(
            matches!(
                actions::upload_file_scoped(
                    API,
                    &fixture.context,
                    "large.txt",
                    vec![0; 8 * 1024 * 1024 + 1],
                    None
                )
                .await,
                Err(SlackActionError::InvalidArgument(_))
            ),
            "oversized upload was not refused"
        );
        let messages = fixture.replies().await?;
        ensure!(
            messages.iter().any(|m| m["files"]
                .as_array()
                .is_some_and(|files| files.iter().any(|f| f["id"] == attachment.id))),
            "file absent from the intended thread"
        );
        let reaction = fixture.reaction_readback().await?;
        ensure!(
            reaction["message"]["reactions"]
                .as_array()
                .is_some_and(|r| r.iter().any(|r| r["name"] == "eyes")),
            "reaction absent from the intended message"
        );
        Ok(())
    }
    .await;
    let cleanup = fixture.cleanup().await;
    if let Err(error) = &cleanup {
        eprintln!("{error:#}");
    }
    result.and(cleanup)
}

#[tokio::test]
async fn native_stream_closes_before_replacement_and_reopens_cleanly() -> Result<()> {
    let mut fixture = Fixture::new().await?;
    let result = async {
        let user = required("TEST_SLACK_USER_ID")?;
        let recipient = fixture
            .call("users.info", json!({"user":user}), false)
            .await?;
        ensure!(
            recipient["user"]["is_bot"] == false && recipient["user"]["deleted"] == false,
            "recipient is not an active test user"
        );
        // chat:write accepts a user-ID destination and returns its DM receipt;
        // this fixture needs no separate conversations.open/im:write grant.
        fixture.context.channel_id = user.clone();
        fixture
            .context
            .extra
            .insert(SLACK_RECIPIENT_USER_ID.into(), user);
        fixture.context.extra.insert(
            SLACK_RECIPIENT_TEAM_ID.into(),
            required("TEST_SLACK_TEAM_ID")?,
        );
        fixture.root().await?;
        let first = fixture
            .surfaces()
            .start_checked(&fixture.context)
            .await
            .map_err(|error| anyhow::anyhow!("stream start: {error}"))?;
        fixture.streams.push(first.clone());
        fixture
            .messages
            .push((fixture.context.channel_id.clone(), first.clone()));
        ensure!(
            fixture
                .surfaces()
                .append(&first, "Text to retract", &fixture.context)
                .await
                == DeliveryResult::Ok,
            "append failed"
        );
        let replacement = format!("{} replacement", fixture.nonce);
        ensure!(
            fixture
                .surfaces()
                .replace(&first, &replacement, &fixture.context)
                .await
                == DeliveryResult::Ok,
            "replacement failed"
        );
        ensure!(
            fixture.surfaces().stop(&first, &fixture.context).await == DeliveryResult::Ok,
            "known closed handle was not idempotent"
        );
        let second = fixture
            .surfaces()
            .start_checked(&fixture.context)
            .await
            .map_err(|error| anyhow::anyhow!("stream start: {error}"))?;
        fixture.streams.push(second.clone());
        fixture
            .messages
            .push((fixture.context.channel_id.clone(), second.clone()));
        ensure!(first != second, "a new stream reused the old handle");
        ensure!(
            fixture
                .surfaces()
                .append(&second, &fixture.nonce, &fixture.context)
                .await
                == DeliveryResult::Ok,
            "second append failed"
        );
        ensure!(
            fixture.surfaces().stop(&second, &fixture.context).await == DeliveryResult::Ok,
            "second stream did not close"
        );
        let messages = fixture.replies().await?;
        ensure!(
            messages
                .iter()
                .any(|m| m["ts"] == first && m["text"] == replacement),
            "replacement was not editable in place"
        );
        ensure!(
            messages
                .iter()
                .any(|m| m["ts"] == second && m["text"] == fixture.nonce),
            "second stream readback missing"
        );
        Ok(())
    }
    .await;
    let cleanup = fixture.cleanup().await;
    if let Err(error) = &cleanup {
        eprintln!("{error:#}");
    }
    result.and(cleanup)
}
