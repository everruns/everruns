//! Batched event appends: one insert per run of durable events, optionally
//! fenced by a session actor lease.

use super::*;

impl EventService {
    /// Emit a batch of typed event requests.
    /// Ephemeral delta events skip PG only when NATS is active; otherwise all go to PG.
    /// Returns the count of successfully processed events.
    ///
    /// # Errors
    /// Returns an error if any event_type doesn't match the data type.
    pub async fn emit_batch(&self, requests: Vec<EventRequest>) -> Result<i32> {
        Ok(self.emit_batch_returning_last(requests).await?.0)
    }

    /// [`Self::emit_batch`], also returning the last event as stored (with its
    /// id and sequence), for a caller that reads it.
    pub async fn emit_batch_returning_last(
        &self,
        requests: Vec<EventRequest>,
    ) -> Result<(i32, Option<Event>)> {
        let (count, events) = self.emit_batch_inner(requests, None).await?;
        Ok((count, events.into_iter().last()))
    }

    /// [`Self::emit_batch_returning_last`] for the holder of a session actor
    /// lease: each run of durable events is stored only while `fence` is
    /// still the holder's live lease on the session, checked in the insert's
    /// transaction (`Database::create_events_fenced`). A stale holder's batch
    /// fails with [`crate::storage::ActorLeaseFenceRejected`] and from that
    /// run on nothing lands. Ephemeral events that skip PostgreSQL are not
    /// log appends and are delivered unchecked.
    pub async fn emit_batch_fenced(
        &self,
        requests: Vec<EventRequest>,
        fence: ActorAppendFence<'_>,
    ) -> Result<(i32, Option<Event>)> {
        let (count, events) = self.emit_batch_inner(requests, Some(fence)).await?;
        Ok((count, events.into_iter().last()))
    }

    /// [`Self::emit_batch_fenced`], returning every event as stored, in order
    /// (each with its id and sequence; an ephemeral one as delivered), for a
    /// caller that answers each append of a batch (an Engine epoch).
    pub async fn emit_batch_fenced_all(
        &self,
        requests: Vec<EventRequest>,
        fence: ActorAppendFence<'_>,
    ) -> Result<(i32, Vec<Event>)> {
        self.emit_batch_inner(requests, Some(fence)).await
    }

    async fn emit_batch_inner(
        &self,
        requests: Vec<EventRequest>,
        fence: Option<ActorAppendFence<'_>>,
    ) -> Result<(i32, Vec<Event>)> {
        let mut prepared = Vec::with_capacity(requests.len());
        for mut request in requests {
            self.prepare_request(&mut request).await?;
            self.attach_human_waits(&mut request, &prepared).await?;
            prepared.push(request);
        }

        let skip_ephemeral = self.event_delivery.supports_ephemeral_skip();
        let mut count = 0i32;
        let mut stored = Vec::with_capacity(prepared.len());

        // Consecutive durable events share one insert; an ephemeral event
        // flushes the run first so delivery keeps the batch's order.
        let mut durable_run = Vec::new();
        for request in prepared {
            count += 1;
            if request.is_ephemeral() && skip_ephemeral {
                if !durable_run.is_empty() {
                    stored.extend(
                        self.emit_durable_run(std::mem::take(&mut durable_run), fence)
                            .await?,
                    );
                }
                stored.push(self.emit_ephemeral(request, listener_wait(fence)).await?);
            } else {
                durable_run.push(request);
            }
        }
        if !durable_run.is_empty() {
            stored.extend(self.emit_durable_run(durable_run, fence).await?);
        }

        Ok((count, stored))
    }
}
