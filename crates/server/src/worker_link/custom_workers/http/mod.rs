//! Request-driven transport for customer-hosted runtimes (including Workers).
//! All execution writes go through CustomWorkerService, never the trusted link.
//! THREAT[TM-DURABLE-014]: response-only invocation authority, DNS-pinned tenant
//! egress, bounded admission/streams, and no automatic external-effects replay.
use super::*;
use crate::storage::EncryptionService;
use anyhow::{Context, Result, ensure};
use everruns_core::{
    EgressRequest, EgressRequestKind, EgressService, EventContext, EventRequest, TurnCancelledData,
    TurnFailedData,
};
use futures::StreamExt;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};

const MAX_LINE: usize = 1024 * 1024;
const MAX_STREAM: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 2048;
const MAX_ORG_INVOCATIONS: i64 = 4;
const MAX_PROCESS_INVOCATIONS: usize = 8;
const DEADLINE: Duration = Duration::from_secs(120);

pub fn validate_endpoint(value: &str) -> Result<()> {
    let url = url::Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Invalid endpoint"
    );
    Ok(())
}

pub fn signature(secret: &str, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn signed_request(url: &str, secret: &str, body: Vec<u8>) -> EgressRequest {
    EgressRequest::new(
        "POST",
        url,
        EgressRequestKind::Other("external_agent".into()),
    )
    .require_dns_pinning()
    .header("Content-Type", "application/json")
    .header("X-Everruns-Signature", signature(secret, &body))
    .timeout_ms(DEADLINE.as_millis() as u64)
    .body(body)
}

pub async fn verify_endpoint(egress: &dyn EgressService, url: &str, secret: &str) -> Result<()> {
    validate_endpoint(url)?;
    let challenge = uuid::Uuid::new_v4().to_string();
    let body = serde_json::to_vec(
        &json!({"protocol":"everruns-worker/v1", "type":"verify", "audience":url, "challenge":challenge, "issued_at":chrono::Utc::now().timestamp()}),
    )?;
    let mut response = egress
        .send_stream(signed_request(url, secret, body).timeout_ms(10_000))
        .await?;
    ensure!(response.status == 200, "Verification rejected");
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(chunk) = response.body.next().await {
            bytes.extend(chunk?);
            ensure!(bytes.len() <= 4096, "Verification response too large");
        }
        anyhow::Ok(())
    })
    .await??;
    let proof: Value = serde_json::from_slice(&bytes)?;
    let supplied = hex::decode(proof["proof"].as_str().context("Missing proof")?)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
    mac.update(challenge.as_bytes());
    mac.verify_slice(&supplied)
        .context("Invalid endpoint proof")?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Endpoint {
    id: uuid::Uuid,
    endpoint_url: String,
    org_id: i64,
    endpoint_secret_encrypted: Vec<u8>,
    adapter_token_encrypted: Vec<u8>,
}

fn request<T>(token: &str, value: T) -> Result<Request<T>> {
    let mut req = Request::new(value);
    req.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse()?);
    Ok(req)
}

/// At most eight invocations per CP process; actor leases arbitrate across CPs.
/// Endpoint selection only visits runnable bound agents, oldest first.
pub async fn run(
    db: Arc<StorageBackend>,
    events: Arc<EventService>,
    encryption: Arc<EncryptionService>,
    egress: Arc<dyn EgressService>,
) {
    let service = CustomWorkerServiceImpl::new(db.clone(), events);
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        while tasks.try_join_next().is_some() {}
        let slots = MAX_PROCESS_INVOCATIONS.saturating_sub(tasks.len());
        if slots > 0 {
            let endpoints = sqlx::query_as::<_, Endpoint>("SELECT w.id, w.org_id, w.endpoint_url, w.endpoint_secret_encrypted, w.adapter_token_encrypted FROM custom_workers w WHERE w.enabled AND w.endpoint_url IS NOT NULL AND (SELECT COUNT(*) FROM external_worker_slots slots WHERE slots.org_id = w.org_id AND slots.expires_at > NOW()) < 4 AND EXISTS(SELECT 1 FROM sessions s JOIN session_actors a ON a.session_id = s.id WHERE s.agent_id = w.agent_id AND a.runnable_since IS NOT NULL AND (a.lease_expires_at IS NULL OR a.lease_expires_at <= NOW())) ORDER BY w.last_invoked_at NULLS FIRST, w.id LIMIT $1")
                .bind(slots as i64).fetch_all(db.pool()).await;
            match endpoints {
                Ok(endpoints) => {
                    for endpoint in endpoints {
                        let svc = service.clone();
                        let encryption = encryption.clone();
                        let egress = egress.clone();
                        tasks.spawn(invoke_and_record(svc, encryption, egress, endpoint));
                    }
                }
                Err(error) => tracing::warn!(%error, "External endpoint scheduling failed"),
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn invoke_and_record(
    svc: CustomWorkerServiceImpl,
    encryption: Arc<EncryptionService>,
    egress: Arc<dyn EgressService>,
    endpoint: Endpoint,
) {
    let id = endpoint.id;
    let version = endpoint.adapter_token_encrypted.clone();
    let result = dispatch(&svc, &encryption, egress.as_ref(), endpoint).await;
    if matches!(result, Ok(false)) {
        return;
    }
    let error = result.err().map(|error| {
        tracing::warn!(deployment = %id, %error, "External agent invocation failed");
        "External execution failed; inspect the endpoint and retry the message".to_string()
    });
    let _ = sqlx::query("UPDATE custom_workers SET last_error = $2, last_invoked_at = NOW() WHERE id = $1 AND adapter_token_encrypted = $3")
        .bind(id).bind(error).bind(version).execute(svc.db.pool()).await;
}

async fn dispatch(
    svc: &CustomWorkerServiceImpl,
    encryption: &EncryptionService,
    egress: &dyn EgressService,
    endpoint: Endpoint,
) -> Result<bool> {
    let Some(slot) = admit(&svc.db, endpoint.org_id).await? else {
        return Ok(false);
    };
    let result = tokio::time::timeout(
        Duration::from_secs(145),
        dispatch_admitted(svc, encryption, egress, endpoint),
    )
    .await
    .context("External dispatch deadline exceeded")
    .and_then(|result| result);
    sqlx::query("DELETE FROM external_worker_slots WHERE id = $1")
        .bind(slot)
        .execute(svc.db.pool())
        .await?;
    result
}

async fn admit(db: &StorageBackend, org: i64) -> Result<Option<uuid::Uuid>> {
    let mut tx = db.tx_pool().begin().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended('external-agent-org:' || $1::bigint::text, 0))").bind(org).fetch_one(&mut *tx).await?;
    if !locked {
        return Ok(None);
    }
    sqlx::query("DELETE FROM external_worker_slots WHERE org_id = $1 AND expires_at <= NOW()")
        .bind(org)
        .execute(&mut *tx)
        .await?;
    let active: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM external_worker_slots WHERE org_id = $1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if active >= MAX_ORG_INVOCATIONS {
        return Ok(None);
    }
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO external_worker_slots(id, org_id, expires_at) VALUES($1,$2,NOW() + INTERVAL '150 seconds')").bind(id).bind(org).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(id))
}

async fn dispatch_admitted(
    svc: &CustomWorkerServiceImpl,
    encryption: &EncryptionService,
    egress: &dyn EgressService,
    endpoint: Endpoint,
) -> Result<bool> {
    validate_endpoint(&endpoint.endpoint_url)?;
    let org = svc
        .db
        .get_organization(endpoint.org_id)
        .await?
        .context("Missing organization")?;
    let scope = everruns_core::EgressScope {
        org_id: Some(org.public_id.parse()?),
        session_id: None,
    };
    let token = encryption.decrypt_to_string(&endpoint.adapter_token_encrypted)?;
    let secret = encryption.decrypt_to_string(&endpoint.endpoint_secret_encrypted)?;
    let holder = uuid::Uuid::new_v4().simple().to_string();
    let lease = svc
        .lease_actors(request(
            &token,
            proto::LeaseActorsRequest {
                holder: holder.clone(),
                limit: 1,
                ..Default::default()
            },
        )?)
        .await?
        .into_inner();
    let Some(actor) = lease.actors.into_iter().next() else {
        return Ok(false);
    };
    let mut journal = Vec::new();
    let mut journal_bytes = 0;
    let mut after = Some(0);
    let mut history_exceeded = false;
    loop {
        let page = svc
            .read_journal(request(
                &token,
                proto::CustomWorkerJournalRequest {
                    journal: Some(proto::ReadActorJournalRequest {
                        session_id: actor.session_id.clone(),
                        after_sequence: after,
                        limit: 20,
                    }),
                    actor_fence: Some(fence(&holder, &actor)),
                },
            )?)
            .await?
            .into_inner();
        for event in page.events {
            after = Some(event.sequence);
            let event = everruns_internal_protocol::proto_event_to_schema(event)?;
            journal_bytes += serde_json::to_vec(&event)?.len();
            if journal_bytes > MAX_STREAM / 2 {
                history_exceeded = true;
                break;
            }
            journal.push(event);
        }
        if history_exceeded || journal.len() > 2000 {
            history_exceeded = true;
            break;
        }
        if !page.has_more {
            break;
        }
    }
    if history_exceeded
        && !journal.iter().any(|e| {
            e.event_type == "input.message"
                && i64::from(e.sequence.unwrap_or_default()) > actor.consumed_seq
        })
    {
        let page = svc
            .read_journal(request(
                &token,
                proto::CustomWorkerJournalRequest {
                    journal: Some(proto::ReadActorJournalRequest {
                        session_id: actor.session_id.clone(),
                        after_sequence: Some(i32::try_from(
                            actor.next_input_seq.saturating_sub(1),
                        )?),
                        limit: 1,
                    }),
                    actor_fence: Some(fence(&holder, &actor)),
                },
            )?)
            .await?
            .into_inner();
        for event in page.events {
            journal.push(everruns_internal_protocol::proto_event_to_schema(event)?);
        }
    }
    let input = journal
        .iter()
        .find(|e| {
            e.event_type == "input.message"
                && i64::from(e.sequence.unwrap_or_default()) > actor.consumed_seq
        })
        .context("No pending input")?;
    let input_seq = i64::from(input.sequence.unwrap_or_default());
    let input_data = serde_json::to_value(&input.data)?;
    let input_id: everruns_contracts::typed_id::MessageId =
        serde_json::from_value(input_data["message"]["id"].clone())?;
    let context = EventContext {
        input_message_id: Some(input_id),
        turn_id: Some(everruns_contracts::typed_id::TurnId::from_uuid(
            input_id.uuid(),
        )),
        ..Default::default()
    };
    let session = input.session_id;
    let terminal = journal.iter().any(|e| {
        e.context.input_message_id == Some(input_id)
            && matches!(
                e.event_type.as_str(),
                "turn.completed" | "turn.failed" | "turn.cancelled"
            )
    });
    if !terminal {
        let mut cancelled =
            actor.cancel_requested && actor.cancel_inbox_seq.is_none_or(|seq| seq >= input_seq);
        let initially_cancelled = cancelled;
        let result = {
            let future = async {
                ensure!(
                    actor.attempts <= 1,
                    "Interrupted external invocation requires explicit retry"
                );
                ensure!(!initially_cancelled, "External invocation cancelled");
                ensure!(!history_exceeded, "External session history limit exceeded");
                let body = serde_json::to_vec(
                    &json!({"protocol":"everruns-worker/v1", "type":"invoke", "audience":endpoint.endpoint_url, "issued_at":chrono::Utc::now().timestamp(), "invocation_id":input_id, "session_id":session, "input":input_data["message"], "input_sequence":input_seq, "journal":journal}),
                )?;
                ensure!(body.len() <= MAX_STREAM, "External context too large");
                let mut response = egress
                    .send_stream(signed_request(&endpoint.endpoint_url, &secret, body).scope(
                        everruns_core::EgressScope {
                            session_id: Some(session),
                            ..scope.clone()
                        },
                    ))
                    .await?;
                ensure!(
                    response.status == 200,
                    "External endpoint rejected invocation"
                );
                let mut buffer = Vec::new();
                let mut total = 0;
                let mut count = 0;
                let mut terminal_event = None;
                while let Some(chunk) = response.body.next().await {
                    let chunk = chunk?;
                    total += chunk.len();
                    ensure!(total <= MAX_STREAM, "External stream too large");
                    buffer.extend(chunk);
                    while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                        ensure!(end <= MAX_LINE, "External event too large");
                        let line: Vec<_> = buffer.drain(..=end).collect();
                        if line.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        count += 1;
                        ensure!(count <= MAX_EVENTS, "External event limit exceeded");
                        ensure!(terminal_event.is_none(), "Event after terminal event");
                        let mut value: Value = serde_json::from_slice(&line)?;
                        ensure!(value.is_object(), "External event must be an object");
                        if value["context"].is_null() {
                            value["context"] = json!({});
                        }
                        ensure!(
                            value["context"].is_object(),
                            "External context must be an object"
                        );
                        value["session_id"] = json!(session);
                        value["ts"] = json!(chrono::Utc::now());
                        // The HTTP connection grants authority to exactly this input.
                        value["context"]["input_message_id"] = json!(input_id);
                        value["context"]["turn_id"] = json!(context.turn_id);
                        let event: EventRequest = serde_json::from_value(value)?;
                        let completed = matches!(
                            event.event_type.as_str(),
                            "turn.completed" | "turn.failed" | "turn.cancelled"
                        );
                        if completed {
                            terminal_event = Some(event);
                        } else {
                            emit(svc, &token, &holder, &actor, event).await?;
                        }
                    }
                    ensure!(buffer.len() <= MAX_LINE, "External event too large");
                }
                ensure!(
                    buffer.iter().all(u8::is_ascii_whitespace) && terminal_event.is_some(),
                    "External stream ended before a terminal event"
                );
                emit(svc, &token, &holder, &actor, terminal_event.unwrap()).await?;
                anyhow::Ok(())
            };
            tokio::pin!(future);
            let mut renew = tokio::time::interval(Duration::from_secs(5));
            let deadline = tokio::time::sleep(DEADLINE);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    result = &mut future => break result,
                    _ = &mut deadline => break Err(anyhow::anyhow!("External execution timed out")),
                    _ = renew.tick() => {
                        let renewed = svc.renew_actor_lease(request(&token, proto::RenewActorLeaseRequest { session_id: actor.session_id.clone(), holder: holder.clone(), fence: actor.fence, ..Default::default() })?).await?.into_inner();
                        ensure!(renewed.held, "External execution lease lost");
                        if renewed.cancel_requested && renewed.cancel_inbox_seq.is_none_or(|seq| seq >= input_seq) { cancelled = true; break Err(anyhow::anyhow!("External execution cancelled")); }
                    }
                }
            }
        };
        if result.is_err() {
            // No automatic replay of customer side effects. A caller can send
            // another message after investigating; a restart seals the attempt.
            let event = if cancelled {
                EventRequest::new(
                    session,
                    context.clone(),
                    TurnCancelledData {
                        turn_id: context.turn_id.unwrap(),
                        reason: Some("Cancelled by caller".into()),
                        usage: None,
                    },
                )
            } else {
                EventRequest::new(session, context.clone(), TurnFailedData { turn_id: context.turn_id.unwrap(), error: "External execution interrupted or rejected. Retry explicitly after checking the endpoint.".into(), error_code: None, error_fields: None, error_disclosure: None })
            };
            emit(svc, &token, &holder, &actor, event).await?;
        }
        svc.release_actor(request(
            &token,
            proto::ReleaseActorRequest {
                session_id: actor.session_id.clone(),
                holder,
                fence: actor.fence,
                consumed_inbox_seq: input_seq,
            },
        )?)
        .await?;
        return result.map(|()| true);
    }
    svc.release_actor(request(
        &token,
        proto::ReleaseActorRequest {
            session_id: actor.session_id,
            holder,
            fence: actor.fence,
            consumed_inbox_seq: input_seq,
        },
    )?)
    .await?;
    Ok(true)
}

fn fence(holder: &str, actor: &proto::LeasedActor) -> proto::ActorFence {
    proto::ActorFence {
        holder: holder.into(),
        fence: actor.fence,
        detached_tool_call_id: None,
    }
}
async fn emit(
    svc: &CustomWorkerServiceImpl,
    token: &str,
    holder: &str,
    actor: &proto::LeasedActor,
    event: EventRequest,
) -> Result<()> {
    svc.emit_event(request(
        token,
        proto::EmitEventRequest {
            event: Some(everruns_internal_protocol::schema_event_request_to_proto(
                &event,
            )),
            actor_fence: Some(fence(holder, actor)),
        },
    )?)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
