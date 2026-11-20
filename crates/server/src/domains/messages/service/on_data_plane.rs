//! Creating a message on a v2 session from the control plane (data plane
//! v2, step 11.6).
//!
//! Decision: a v2 session's text messages go straight to its cell through
//! the edge. The ones that reach the control plane are those the edge
//! passes here because they refer to uploads, and the platform's own (a
//! reaction note). Their uploads are copied into the session's files on
//! the cell first (`data_plane::attachments`), then the message is
//! admitted there with its parts, under the client's message id when it
//! gave one, so a retry is the same message. The message's metadata
//! carries the platform's entries (its origin), as on v1.

use everruns_contracts::session_commands;
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::ContentPart;
use serde_json::{Value, json};

use super::CreateMessageContext;
use crate::domains::data_plane::{ControlPlane, attachments, for_org, org_public};
use crate::domains::messages::types::{
    CreateMessageRequest, Message, MessageDelivery, MessageRole,
};
use crate::storage::StorageBackend;

pub(super) async fn create(
    db: &Arc<StorageBackend>,
    cp: &ControlPlane,
    ctx: &CreateMessageContext,
    req: CreateMessageRequest,
    platform_metadata: Vec<(String, Value)>,
) -> Result<Message> {
    let session = SessionId::from_uuid(ctx.session_id);
    // A v1 session moving onto its data plane takes messages again once its
    // import is done (`data_plane::moves`).
    if db.dp_session_moving(&session.to_string()).await? {
        return Err(crate::errors::ConflictError::new(
            "The session is moving to the data plane; send the message again in a moment",
        )
        .into());
    }
    let content: Vec<ContentPart> = req
        .message
        .content
        .into_iter()
        .map(ContentPart::from)
        .collect();
    let plane = for_org(cp, ctx.org_id).await?;
    let org = org_public(ctx.org_id);
    attachments::stage(db, plane.as_ref(), &org, ctx.org_id, session, &content).await?;
    let id = req
        .client_message_id
        .map(MessageId::from_uuid)
        .unwrap_or_default();
    let mut metadata: HashMap<String, Value> = req.metadata.unwrap_or_default();
    metadata.extend(platform_metadata);
    let metadata = (!metadata.is_empty()).then_some(metadata);
    let reply = plane
        .session_command(
            &org,
            session,
            &session_commands::MESSAGES,
            &json!({
                "id": id,
                "parts": content,
                "metadata": metadata,
                "external_actor": req.external_actor,
                // The person who sent it, so the turn's platform tools act
                // as them; none for a platform message (the cell's
                // `senders.rs`), as v1 records the invocation.
                "sender": ctx.event_metadata.is_none().then_some(ctx.user_id).flatten(),
            }),
        )
        .await?;
    let delivery = match reply["disposition"].as_str() {
        Some("steered") => MessageDelivery::Steered,
        _ => MessageDelivery::Started,
    };
    Ok(Message {
        id,
        session_id: session,
        sequence: 0,
        role: MessageRole::User,
        content,
        phase: None,
        phase_source: None,
        controls: req.controls,
        metadata,
        external_actor: req.external_actor,
        delivery: Some(delivery),
        created_at: Utc::now(),
    })
}
