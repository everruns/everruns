//! The channel host's store on PostgreSQL (migration 210).
//!
//! Decision: the host names a server channel by its row's internal id (see
//! [`super::resolver`]), so every key here is that id. A name that is not one
//! cannot have rows: lookups find nothing and writes are refused.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::channel::{ChannelError, DeliveryTarget};
use everruns_core::channel_runtime::{ChannelStore, PendingDelivery};
use uuid::Uuid;

use crate::storage::{ChannelPendingDeliveryRow, StorageBackend};

/// [`ChannelStore`] over the server's database.
#[derive(Clone)]
pub struct PgChannelStore {
    db: Arc<StorageBackend>,
}

impl PgChannelStore {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

fn channel_id(channel: &str) -> Result<Uuid, ChannelError> {
    Uuid::parse_str(channel)
        .map_err(|_| ChannelError::NotFound(format!("channel {channel} is not a server channel")))
}

fn session_uuid(session_id: &str) -> Result<Uuid, ChannelError> {
    session_id
        .parse::<everruns_contracts::typed_id::SessionId>()
        .map(|id| id.uuid())
        .map_err(|_| ChannelError::BadRequest(format!("bad session id {session_id}")))
}

fn session_string(session_id: Uuid) -> String {
    everruns_contracts::typed_id::SessionId::from_uuid(session_id).to_string()
}

#[async_trait]
impl ChannelStore for PgChannelStore {
    async fn session_for(&self, channel: &str, key: &str) -> Result<Option<String>, ChannelError> {
        let Ok(channel_id) = channel_id(channel) else {
            return Ok(None);
        };
        Ok(self
            .db
            .channel_binding(channel_id, key)
            .await?
            .map(session_string))
    }

    async fn bind(
        &self,
        channel: &str,
        key: &str,
        session_id: &str,
    ) -> Result<String, ChannelError> {
        let bound = self
            .db
            .bind_channel_session(channel_id(channel)?, key, session_uuid(session_id)?)
            .await?;
        Ok(session_string(bound))
    }

    async fn first_sighting(&self, channel: &str, dedup_key: &str) -> Result<bool, ChannelError> {
        Ok(self
            .db
            .record_channel_delivery_key(channel_id(channel)?, dedup_key)
            .await?)
    }

    async fn save_pending(
        &self,
        pending: &PendingDelivery,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        let row = ChannelPendingDeliveryRow {
            channel_id: channel_id(&pending.channel)?,
            session_id: session_uuid(&pending.session_id)?,
            input_message_id: pending.input_message_id.clone(),
            target: serde_json::to_value(&pending.target).map_err(anyhow::Error::from)?,
            after_sequence: pending.after_sequence,
        };
        Ok(self
            .db
            .save_channel_pending_delivery(&row, holder, lease)
            .await?)
    }

    async fn renew_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        Ok(self
            .db
            .renew_channel_pending_delivery(
                session_uuid(session_id)?,
                input_message_id,
                holder,
                lease,
            )
            .await?)
    }

    async fn clear_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<(), ChannelError> {
        Ok(self
            .db
            .clear_channel_pending_delivery(session_uuid(session_id)?, input_message_id)
            .await?)
    }

    async fn claim_pending(
        &self,
        holder: &str,
        lease: Duration,
    ) -> Result<Vec<PendingDelivery>, ChannelError> {
        let rows = self
            .db
            .claim_channel_pending_deliveries(holder, lease)
            .await?;
        let mut pending = Vec::with_capacity(rows.len());
        for row in rows {
            match serde_json::from_value::<DeliveryTarget>(row.target) {
                Ok(target) => pending.push(PendingDelivery {
                    channel: row.channel_id.to_string(),
                    session_id: session_string(row.session_id),
                    input_message_id: row.input_message_id,
                    target,
                    after_sequence: row.after_sequence,
                }),
                // Unreadable now means unreadable on every later claim too.
                Err(error) => {
                    tracing::warn!(%error, session_id = %row.session_id, "channel host: dropping a pending delivery with an unreadable target");
                    self.db
                        .clear_channel_pending_delivery(row.session_id, &row.input_message_id)
                        .await?;
                }
            }
        }
        Ok(pending)
    }

    async fn release_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
    ) -> Result<(), ChannelError> {
        Ok(self
            .db
            .release_channel_pending_delivery(session_uuid(session_id)?, input_message_id, holder)
            .await?)
    }
}
