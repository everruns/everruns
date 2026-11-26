//! Credentials are resolved at each operation. Body fetches are bounded and
//! persisted; a provider summary is never mistaken for an empty message.
use crate::storage::{EncryptionService, StorageBackend};
use channelkit_agentmail::{AgentMail, EmailMessage};
use std::sync::Arc;
use uuid::Uuid;

pub async fn client(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org: i64,
    inbox: Uuid,
) -> anyhow::Result<AgentMail> {
    let (account,provider_id):(Uuid,String)=sqlx::query_as("SELECT account_id,provider_inbox_id FROM email_inboxes WHERE org_id=$1 AND id=$2 AND enabled")
        .bind(org).bind(inbox).fetch_one(db.pool()).await?;
    let connection = db
        .get_organization_connection(org, account)
        .await?
        .filter(|c| c.provider == "agentmail_inbox")
        .ok_or_else(|| anyhow::anyhow!("inbox account unavailable"))?;
    let key = encryption
        .ok_or_else(|| anyhow::anyhow!("encryption unavailable"))?
        .decrypt_to_string(
            connection
                .access_token_encrypted
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("inbox credential unavailable"))?,
        )?;
    Ok(AgentMail::new(
        key,
        provider_id,
        Arc::new(channelkit::http::ReqwestTransport::new()?),
    )?)
}

pub async fn hydrate(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org: i64,
    inbox: Uuid,
    id: Uuid,
) -> anyhow::Result<EmailMessage> {
    let q = super::queue::EmailQueue::new(db.pool().clone());
    let old = q
        .message(org, inbox, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("message unavailable"))?;
    if (old.text.is_some() || old.html.is_some())
        && old
            .agent_armor
            .as_ref()
            .is_none_or(|a| a.status != "pending")
    {
        return Ok(old);
    }
    let mail = client(db, encryption, org, inbox)
        .await?
        .get_message(&old.message_id)
        .await?;
    anyhow::ensure!(
        mail.thread_id == old.thread_id,
        "provider changed thread identity"
    );
    anyhow::ensure!(
        mail.text.is_some() || mail.html.is_some(),
        "message body unavailable"
    );
    let body = serde_json::to_value(&mail)?;
    let size = serde_json::to_vec(&body)?.len();
    anyhow::ensure!(size <= 1_048_576, "message exceeds storage limit");
    let mut tx = db.pool().begin().await?;
    sqlx::query("SELECT org_id FROM organizations WHERE org_id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(stored_bytes),0)::bigint FROM email_intake WHERE org_id=$1 AND id<>$2",
    )
    .bind(org)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    anyhow::ensure!(
        total + size as i64 <= 256 * 1024 * 1024,
        "email storage quota"
    );
    sqlx::query("UPDATE email_intake SET message=$4,stored_bytes=$5,updated_at=now() WHERE org_id=$1 AND inbox_id=$2 AND id=$3").bind(org).bind(inbox).bind(id).bind(body).bind(size as i32).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(mail)
}

/// Only a single valid From mailbox is usable; a display name is never identity.
pub fn sender(mail: &EmailMessage) -> anyhow::Result<&str> {
    let raw = mail.from.trim();
    let value = if let Some((display, tail)) = raw.rsplit_once('<') {
        anyhow::ensure!(
            !display.contains(['<', '>']) && !display.chars().any(char::is_control),
            "ambiguous sender"
        );
        tail.strip_suffix('>')
            .ok_or_else(|| anyhow::anyhow!("invalid sender"))?
    } else {
        raw
    };
    channelkit_agentmail::mailbox(value)?;
    anyhow::ensure!(!value.contains([',', ';', ':', '"']), "ambiguous sender");
    Ok(value)
}

/// Domain authentication is evidence about email, never an Everruns login or a
/// cross-channel account link. Missing evidence fails closed for automation.
pub fn admission(mail: &EmailMessage) -> Result<(), &'static str> {
    let from = sender(mail).map_err(|_| "invalid_sender")?;
    let domain = from.rsplit_once('@').ok_or("invalid_sender")?.1;
    if mail.labels.iter().any(|s| {
        matches!(
            s.as_str(),
            "spam" | "blocked" | "unauthenticated" | "agent-armor-held" | "sent"
        )
    }) {
        return Err("provider_hold");
    }
    if let Some(armor) = &mail.agent_armor {
        if armor.status != "complete" || armor.verdict.as_deref() != Some("clear") {
            return Err("provider_security_hold");
        }
    }
    for (key, value) in &mail.headers {
        if (key.eq_ignore_ascii_case("auto-submitted") && !value.eq_ignore_ascii_case("no"))
            || key.eq_ignore_ascii_case("list-id")
            || (key.eq_ignore_ascii_case("precedence")
                && ["bulk", "list", "junk"]
                    .iter()
                    .any(|v| value.eq_ignore_ascii_case(v)))
        {
            return Err("automated_or_list_mail");
        }
    }
    let auth = mail
        .authentication_results
        .as_ref()
        .ok_or("sender_authentication_unavailable")?;
    let aligned = [
        (&auth.dmarc, &auth.dmarc_domain),
        (&auth.dkim, &auth.dkim_domain),
        (&auth.spf, &auth.spf_domain),
    ]
    .iter()
    .any(|(verdict, authenticated)| {
        verdict.as_deref() == Some("pass")
            && authenticated
                .as_deref()
                .is_some_and(|d| d.eq_ignore_ascii_case(domain))
    });
    if !aligned {
        return Err("sender_authentication_unavailable");
    }
    Ok(())
}

pub fn clean(mail: &EmailMessage) -> anyhow::Result<markkit::email::EmailView> {
    // Local cleanup retains provenance. Provider-extracted text can omit a
    // forwarded request, so it is never the only representation kept or gated.
    Ok(markkit::email::extract_body(
        markkit::email::EmailBody {
            text: mail.text.as_deref(),
            html: mail.html.as_deref(),
        },
        markkit::Limits::default()
            .max_input_size(1_048_576)
            .max_output_size(262_144),
    )?)
}

/// One recovery page per minute per inbox. The page cursor and fixed lower
/// bound survive restarts; overlapping windows plus receipt dedup cover races.
pub async fn reconcile(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org: i64,
    inbox: Uuid,
) -> anyhow::Result<usize> {
    let row:Option<(chrono::DateTime<chrono::Utc>,Option<String>,chrono::DateTime<chrono::Utc>)>=sqlx::query_as("UPDATE email_inboxes SET last_polled_at=now(),poll_started_at=coalesce(poll_started_at,now()) WHERE org_id=$1 AND id=$2 AND enabled AND (last_polled_at IS NULL OR last_polled_at<now()-interval '1 minute') RETURNING poll_after,poll_page,poll_started_at")
        .bind(org).bind(inbox).fetch_optional(db.pool()).await?;
    let Some((after, cursor, started)) = row else {
        return Ok(0);
    };
    let client = client(db, encryption, org, inbox).await?;
    let page = client
        .list_received(&after.to_rfc3339(), cursor.as_deref())
        .await?;
    let queue = super::queue::EmailQueue::new(db.pool().clone());
    let count = page.messages.len();
    for mail in page.messages {
        queue
            .ingest(
                org,
                inbox,
                &channelkit_agentmail::EmailEvent {
                    event_id: format!("reconcile:{}", mail.message_id),
                    event_type: "message.received".into(),
                    message: mail,
                },
            )
            .await?;
    }
    // Failed/cancelled ingestion leaves the same cursor for the next attempt.
    sqlx::query("UPDATE email_inboxes SET poll_page=$3,poll_after=CASE WHEN $3::text IS NULL THEN $4-interval '1 minute' ELSE poll_after END,poll_started_at=CASE WHEN $3::text IS NULL THEN NULL ELSE poll_started_at END WHERE org_id=$1 AND id=$2 AND poll_started_at=$4 AND poll_page IS NOT DISTINCT FROM $5")
        .bind(org).bind(inbox).bind(page.next_page_token).bind(started).bind(cursor).execute(db.pool()).await?;
    Ok(count)
}
