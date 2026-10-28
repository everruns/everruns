//! The worker's `LoadMessages` read: served from [`super::history_cache`]
//! when it can, otherwise one windowed server read.

use super::{
    GrpcAdapter, Result, RuntimeMessage, SessionId, grpc_status_to_error, proto,
    proto_message_to_message, uuid_to_proto,
};

impl GrpcAdapter {
    pub(super) async fn load_with_message_limit(
        &self,
        session_id: SessionId,
        message_limit: Option<i32>,
        after_sequence: Option<i64>,
    ) -> Result<(Vec<RuntimeMessage>, usize, Option<i64>)> {
        let history = &self.client.history;
        if after_sequence.is_none() && history.enabled() {
            let cached = history.get(session_id.uuid());
            // A cold "latest N" read stays a windowed server read: warming
            // the cache would load the whole history for one window.
            if cached.is_some() || message_limit.is_none() {
                let after = cached.as_ref().map(|c| c.source_sequence);
                let (appended, _, source_sequence, bytes) = match self
                    .load_from_server(session_id, None, after, after.is_some())
                    .await
                {
                    Ok(loaded) => loaded,
                    Err(e) => {
                        history.forget(session_id.uuid());
                        return Err(e);
                    }
                };
                let source_sequence = source_sequence.or(after);
                let messages = history.extend(
                    session_id.uuid(),
                    cached.as_deref(),
                    appended,
                    bytes,
                    source_sequence.unwrap_or(0),
                );
                let total_count = messages.len();
                return Ok((messages, total_count, source_sequence));
            }
        }
        let (messages, total_count, source_sequence, _) = self
            .load_from_server(session_id, message_limit, after_sequence, false)
            .await?;
        Ok((messages, total_count, source_sequence))
    }

    /// One `LoadMessages` call: the messages, the session's message count,
    /// the sequence read up to, and the wire size of the messages. A cache
    /// extension sets `skip_total_count`, since it knows the count, which
    /// saves a count over the whole session.
    async fn load_from_server(
        &self,
        session_id: SessionId,
        message_limit: Option<i32>,
        after_sequence: Option<i64>,
        skip_total_count: bool,
    ) -> Result<(Vec<RuntimeMessage>, usize, Option<i64>, u64)> {
        use prost::Message as _;
        let mut client = self.client.inner.client();

        let request = proto::LoadMessagesRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            message_limit,
            after_sequence,
            skip_total_count: Some(skip_total_count),
        };

        let response = client
            .load_messages(request)
            .await
            .map_err(grpc_status_to_error)?;
        let response = response.into_inner();
        let total_count = if response.total_count > 0 {
            response.total_count as usize
        } else {
            response.messages.len()
        };

        let bytes = response
            .messages
            .iter()
            .map(|m| m.encoded_len() as u64)
            .sum();
        let messages = response
            .messages
            .into_iter()
            .map(proto_message_to_message)
            .collect::<Result<Vec<_>>>()?;

        Ok((messages, total_count, response.source_sequence, bytes))
    }
}
