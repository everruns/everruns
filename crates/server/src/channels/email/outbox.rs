//! Immutable send intent plus a compare-and-set handoff. A timeout, process
//! crash or malformed receipt becomes unknown, never an automatic resend.
use crate::storage::{EncryptionService, StorageBackend};
use channelkit_agentmail::EmailSend;
use std::sync::Arc;
use uuid::Uuid;

pub async fn send(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org: i64,
    inbox: Uuid,
    id: Uuid,
) -> anyhow::Result<&'static str> {
    // Resolve current credentials before claiming. Revocation is immediate for
    // queued work; no credential is embedded in a pending delivery.
    let client = super::provider::client(db, encryption, org, inbox).await?;
    send_with_client(db, org, inbox, id, client).await
}

async fn send_with_client(
    db: &StorageBackend,
    org: i64,
    inbox: Uuid,
    id: Uuid,
    client: channelkit_agentmail::AgentMail,
) -> anyhow::Result<&'static str> {
    let mut tx = everruns_durable::begin_transaction(db.pool()).await?;
    sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
    let row:Option<(serde_json::Value,String,String)>=sqlx::query_as("SELECT o.body,m.message_id,o.state FROM email_outbox o JOIN email_intake m ON m.id=o.intake_id AND m.org_id=o.org_id AND m.inbox_id=o.inbox_id WHERE o.org_id=$1 AND o.inbox_id=$2 AND o.id=$3 AND m.state='completed' AND (o.authorized OR o.gate_state='passed') AND o.available_at<=now() AND EXISTS(SELECT 1 FROM email_inboxes i WHERE i.id=o.inbox_id AND i.org_id=o.org_id AND i.enabled) FOR UPDATE OF o")
        .bind(org).bind(inbox).bind(id).fetch_optional(&mut *tx).await?;
    let (body, reply_to, state) = row.ok_or_else(|| anyhow::anyhow!("outbox item unavailable"))?;
    if state == "sent" {
        everruns_durable::commit_transaction(tx).await?;
        return Ok("sent");
    }
    anyhow::ensure!(
        state == "held",
        "send is already claimed or needs reconciliation"
    );
    let message: EmailSend = serde_json::from_value(body)?;
    message.validate()?;
    if !super::queue::reserve(&mut tx, org, "send", 120, 3600).await? {
        sqlx::query("UPDATE email_outbox SET available_at=date_trunc('hour',now())+interval '1 hour' WHERE id=$1").bind(id).execute(&mut *tx).await?;
        everruns_durable::commit_transaction(tx).await?;
        return Ok("held");
    }
    sqlx::query("UPDATE email_outbox SET state='sending',updated_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    everruns_durable::commit_transaction(tx).await?;
    let (state, receipt, delay) = match client
        .send(&message, Some(&reply_to), &id.to_string())
        .await
    {
        Ok(receipt) => ("sent", Some(receipt.message_id), 0i64),
        Err(channelkit::Error::DeliveryUnknown) => ("unknown", None, 0),
        Err(channelkit::Error::RateLimited { retry_after }) => {
            ("held", None, retry_after.as_secs().clamp(1, 86400) as i64)
        }
        Err(error) => {
            tracing::warn!(%id,%error,"Email provider refused send");
            ("failed", None, 0)
        }
    };
    // If this write fails, recovery sees 'sending' and holds it as unknown.
    sqlx::query("UPDATE email_outbox SET state=$4,provider_message_id=$5,available_at=now()+($6::bigint * interval '1 second'),updated_at=now() WHERE org_id=$1 AND inbox_id=$2 AND id=$3 AND state='sending'")
        .bind(org).bind(inbox).bind(id).bind(state).bind(receipt).bind(delay).execute(db.pool()).await?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Wire {
        calls: AtomicUsize,
        first: u16,
    }
    #[async_trait::async_trait]
    impl channelkit::http::Transport for Wire {
        async fn execute(
            &self,
            request: channelkit::http::Request,
        ) -> channelkit::Result<channelkit::http::Response> {
            let count = self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(request.headers.contains_key("idempotency-key"));
            Ok(channelkit::http::Response {
                status: if count == 0 { self.first } else { 200 },
                headers: std::collections::BTreeMap::from([("retry-after".into(), "1".into())]),
                body: br#"{"message_id":"sent","thread_id":"thread"}"#.to_vec(),
            })
        }
    }
    async fn intent(db: &StorageBackend, org: i64, inbox: Uuid) -> (Uuid, Uuid) {
        let intake = Uuid::now_v7();
        let out = Uuid::now_v7();
        sqlx::query("INSERT INTO email_intake(id,org_id,inbox_id,event_id,message_id,thread_id,sender,subject,preview,message,stored_bytes,state) VALUES($1,$2,$3,'event','message','thread','a@example.com','subject','preview','{}',2,'completed')").bind(intake).bind(org).bind(inbox).execute(db.pool()).await.unwrap();
        let body = EmailSend {
            to: vec!["a@example.com".into()],
            cc: vec![],
            subject: "Re: subject".into(),
            text: "Answer".into(),
            html: Some("<p>Answer</p>".into()),
        };
        sqlx::query("INSERT INTO email_outbox(id,org_id,inbox_id,intake_id,session_id,output_key,body) VALUES($1,$2,$3,$4,$5,'stable',$6)").bind(out).bind(org).bind(inbox).bind(intake).bind(Uuid::now_v7()).bind(json!(body)).execute(db.pool()).await.unwrap();
        (intake, out)
    }
    #[tokio::test]
    async fn email_outbox_defers_known_rate_limits_but_never_retries_unknown_acceptance() {
        for first in [429, 500] {
            let (db, org, inbox) = super::super::tests::fixture().await;
            let (intake, id) = intent(&db, org, inbox).await;
            let wire = Arc::new(Wire {
                calls: AtomicUsize::new(0),
                first,
            });
            let client = || {
                channelkit_agentmail::AgentMail::new(
                    "test".into(),
                    "support@agentmail.to".into(),
                    wire.clone(),
                )
                .unwrap()
            };
            assert!(
                send_with_client(&db, org, inbox, id, client())
                    .await
                    .is_err()
            );
            assert_eq!(wire.calls.load(Ordering::SeqCst), 0);
            sqlx::query("UPDATE email_outbox SET authorized=true WHERE id=$1")
                .bind(id)
                .execute(db.pool())
                .await
                .unwrap();
            sqlx::query("UPDATE email_intake SET state='running' WHERE id=$1")
                .bind(intake)
                .execute(db.pool())
                .await
                .unwrap();
            assert!(
                send_with_client(&db, org, inbox, id, client())
                    .await
                    .is_err()
            );
            sqlx::query("UPDATE email_intake SET state='completed' WHERE id=$1")
                .bind(intake)
                .execute(db.pool())
                .await
                .unwrap();
            let state = send_with_client(&db, org, inbox, id, client())
                .await
                .unwrap();
            assert_eq!(state, if first == 429 { "held" } else { "unknown" });
            sqlx::query(
                "UPDATE email_outbox SET available_at=now()-interval '1 second' WHERE id=$1",
            )
            .bind(id)
            .execute(db.pool())
            .await
            .unwrap();
            if first == 429 {
                assert_eq!(
                    send_with_client(&db, org, inbox, id, client())
                        .await
                        .unwrap(),
                    "sent"
                );
                assert_eq!(
                    send_with_client(&db, org, inbox, id, client())
                        .await
                        .unwrap(),
                    "sent"
                );
                assert_eq!(wire.calls.load(Ordering::SeqCst), 2);
            } else {
                assert!(
                    send_with_client(&db, org, inbox, id, client())
                        .await
                        .is_err()
                );
                assert_eq!(wire.calls.load(Ordering::SeqCst), 1);
            }
        }
    }
}
