//! The ChannelHost remains the sole session/delivery coordinator. Email output
//! first enters an immutable outbox; review authorization is an explicit host
//! action, never something inbound mail or model arguments can manufacture.
use super::{EmailChannelConfig, compose};
use crate::channels::host::ServerChannelScope;
use crate::storage::StorageBackend;
use async_trait::async_trait;
use everruns_contracts::runtime::channel::{
    ChannelDeliveryAdapter, ChannelDriver, ChannelError, ChannelRequest, DeliveryContext,
    DeliveryResult, Inbound, OutboundChannelMessage,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

pub struct EmailDriver {
    pub db: Arc<StorageBackend>,
    pub scope: ServerChannelScope,
    pub config: EmailChannelConfig,
}
#[async_trait]
impl ChannelDriver for EmailDriver {
    async fn receive(&self, _: &ChannelRequest) -> Result<Inbound, ChannelError> {
        // Only the durable intake dispatcher may submit inputs through
        // send_in_conversation. The public route verifies and persists first.
        Err(ChannelError::Unauthorized(
            "Email inputs require durable intake".into(),
        ))
    }
}
#[async_trait]
impl ChannelDeliveryAdapter for EmailDriver {
    fn platform(&self) -> &str {
        "email"
    }
    fn requires_checkpoint(&self) -> bool {
        true
    }
    fn idempotent_replies(&self) -> bool {
        true
    }
    fn aggregate_replies(&self) -> bool {
        true
    }
    fn durable_intake(&self) -> bool {
        true
    }
    async fn turn_finished(&self, context: &DeliveryContext, event_type: &str) -> DeliveryResult {
        let Some(id) = context
            .extra
            .get("email_intake_id")
            .and_then(|s| Uuid::parse_str(s).ok())
        else {
            return DeliveryResult::PermanentError("Missing intake identity".into());
        };
        match sqlx::query("UPDATE email_intake SET state=$4,updated_at=now() WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND state='running'").bind(self.scope.context.org_id).bind(self.config.inbox_id).bind(id).bind(if event_type=="turn.completed" {"completed"}else{"failed"}).execute(self.db.pool()).await {
            Ok(_)=>DeliveryResult::Ok,Err(_)=>DeliveryResult::TransientError("Email completion checkpoint unavailable".into())
        }
    }
    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        match self.stage(message, context).await {
            Ok(Some(receipt)) => {
                if let Some(sink) = &context.receipts
                    && sink.confirmed(&receipt).await.is_err()
                {
                    return DeliveryResult::Unknown {
                        confirmed: vec![receipt],
                    };
                }
                DeliveryResult::Ok
            }
            Ok(None) => DeliveryResult::Ok,
            Err(error) => {
                tracing::warn!(%error,"Email reply staging failed");
                DeliveryResult::TransientError("Email reply could not be staged".into())
            }
        }
    }
}
impl EmailDriver {
    async fn stage(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> anyhow::Result<Option<String>> {
        let intake: Uuid = context
            .extra
            .get("email_intake_id")
            .ok_or_else(|| anyhow::anyhow!("missing intake identity"))?
            .parse()?;
        let org = self.scope.context.org_id;
        let mail:serde_json::Value=sqlx::query_scalar("SELECT message FROM email_intake WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND state IN ('running','completed')").bind(org).bind(self.config.inbox_id).bind(intake).fetch_one(self.db.pool()).await?;
        let source: channelkit_agentmail::EmailMessage = serde_json::from_value(mail)?;
        anyhow::ensure!(source.thread_id == context.thread_ref, "thread mismatch");
        // Header text remains untrusted; never accept extra recipients from a
        // model or Reply-To. Review shows the exact sender mailbox used here.
        let recipient = super::provider::sender(&source)?;
        let previous:Option<Option<String>>=sqlx::query_scalar("SELECT provider_message_id FROM email_outbox WHERE org_id=$1 AND inbox_id=$2 AND intake_id=$3").bind(org).bind(self.config.inbox_id).bind(intake).fetch_optional(self.db.pool()).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let template = if let Some(id) = self.config.reply_template_id {
            let (name,subject,html,text):(String,String,Option<String>,Option<String>)=sqlx::query_as("SELECT name,subject,html,text_body FROM email_templates WHERE org_id=$1 AND id=$2 ORDER BY version DESC LIMIT 1").bind(org).bind(id).fetch_one(self.db.pool()).await?;
            compose::EmailTemplate {
                name,
                subject,
                html,
                text,
            }
        } else {
            compose::EmailTemplate {
                name: "Reply".into(),
                subject: "{{subject}}".into(),
                text: Some("{{body}}".into()),
                html: None,
            }
        };
        let rendered = compose::render(
            &template,
            &std::collections::BTreeMap::from([
                (
                    "subject".into(),
                    format!("Re: {}", super::queue::excerpt(&source.subject, 900)),
                ),
                ("body".into(), message.text.clone()),
            ]),
        )?;
        let body = channelkit_agentmail::EmailSend {
            to: vec![recipient.to_string()],
            cc: vec![],
            // AgentMail replies retain the original thread subject.
            subject: format!("Re: {}", super::queue::excerpt(&source.subject, 900)),
            text: rendered.text,
            html: Some(rendered.html),
        };
        body.validate()?;
        let value = serde_json::to_value(body)?;
        let digest = hex::encode(Sha256::digest(serde_json::to_vec(&value)?));
        let key = format!(
            "{intake}:{}:{digest}",
            message.correlation_id.as_deref().unwrap_or("")
        );
        let receipt:Option<String>=sqlx::query_scalar("INSERT INTO email_outbox (id,org_id,inbox_id,intake_id,session_id,output_key,body) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (intake_id) DO UPDATE SET output_key=email_outbox.output_key RETURNING provider_message_id")
            .bind(Uuid::now_v7()).bind(org).bind(self.config.inbox_id).bind(intake).bind(message.session_id.uuid()).bind(key).bind(value).fetch_one(self.db.pool()).await?;
        Ok(receipt)
    }
}
