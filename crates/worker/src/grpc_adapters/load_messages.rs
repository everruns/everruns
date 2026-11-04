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
            let limit = message_limit.map(|limit| usize::try_from(limit).unwrap_or(0));
            if let Some(cached) = history.get(session_id.uuid()) {
                let (appended, _, source_sequence, bytes) = match self
                    .load_from_server(session_id, None, Some(cached.source_sequence), true)
                    .await
                {
                    Ok(loaded) => loaded,
                    Err(e) => {
                        history.forget(session_id.uuid());
                        return Err(e);
                    }
                };
                // A cached window too short for this read (a wider window,
                // or a full read) is read afresh below.
                if cached.covers(appended.len(), limit) {
                    let source_sequence = source_sequence.or(Some(cached.source_sequence));
                    let total_count = cached.total_count + appended.len();
                    let messages = history.extend(
                        session_id.uuid(),
                        Some(&cached),
                        appended,
                        bytes,
                        source_sequence.unwrap_or(0),
                        0,
                    );
                    return Ok((messages, total_count, source_sequence));
                }
            }
            // Cold: one read, a window or the whole history as asked, which
            // the cache keeps with the session's message count.
            let (messages, total_count, source_sequence, bytes) = match self
                .load_from_server(session_id, message_limit, None, false)
                .await
            {
                Ok(loaded) => loaded,
                Err(e) => {
                    history.forget(session_id.uuid());
                    return Err(e);
                }
            };
            let total_count = if message_limit.is_none() {
                messages.len()
            } else {
                total_count.max(messages.len())
            };
            let messages = history.extend(
                session_id.uuid(),
                None,
                messages,
                bytes,
                source_sequence.unwrap_or(0),
                total_count,
            );
            return Ok((messages, total_count, source_sequence));
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
