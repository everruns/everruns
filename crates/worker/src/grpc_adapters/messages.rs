//! Bounded journal and search reads over the worker transport.

use super::*;

#[async_trait]
impl MessageRetriever for GrpcAdapter {
    async fn get(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<Option<RuntimeMessage>> {
        let mut client = self.client.inner.client();

        let request = proto::GetMessageRequest {
            session_id: Some(uuid_to_proto(session_id.uuid())),
            message_id: Some(uuid_to_proto(message_id.uuid())),
        };

        let response = client
            .get_message(request)
            .await
            .map_err(grpc_status_to_error)?;

        response
            .into_inner()
            .message
            .map(proto_message_to_message)
            .transpose()
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<RuntimeMessage>> {
        let (messages, _, _) = self.load_with_message_limit(session_id, None, None).await?;
        Ok(messages)
    }

    async fn load_filtered(
        &self,
        query: crate::core::message_filter::MessageQuery,
    ) -> Result<Vec<RuntimeMessage>> {
        Ok(self.load_filtered_history(query).await?.messages)
    }

    async fn load_filtered_history(
        &self,
        query: crate::core::message_filter::MessageQuery,
    ) -> Result<MessageHistory> {
        // Bounded continuity pages and indexed searches bypass the tail cache.
        if query.oldest_first
            || query.before_sequence.is_some()
            || query
                .filters
                .iter()
                .any(|f| matches!(f, MessageFilter::Search(_)))
        {
            let search = query.filters.iter().find_map(|f| match f {
                MessageFilter::Search(s) => Some(s.clone()),
                _ => None,
            });
            if query
                .filters
                .iter()
                .any(|f| !matches!(f, MessageFilter::Search(_)))
            {
                return Err(AgentLoopError::store("unsupported bounded history filter"));
            }
            let response = self
                .client
                .inner
                .client()
                .load_messages(proto::LoadMessagesRequest {
                    session_id: Some(uuid_to_proto(query.session_id.uuid())),
                    // A missing limit means the whole selected suffix. Capping
                    // it here would silently drop unsummarized source records.
                    message_limit: query
                        .limit
                        .map(|limit| limit.clamp(0, i32::MAX as i64) as i32),
                    after_sequence: query.after_sequence,
                    before_sequence: query.before_sequence,
                    oldest_first: query.oldest_first,
                    offset: query.offset,
                    search,
                    skip_total_count: Some(true),
                })
                .await
                .map_err(grpc_status_to_error)?
                .into_inner();
            let messages = response
                .messages
                .into_iter()
                .map(proto_message_to_message)
                .collect::<Result<Vec<_>>>()?;
            let sequences = response
                .message_sequences
                .into_iter()
                .map(|(id, seq)| {
                    id.parse().map(|id| (id, seq)).map_err(|e| {
                        AgentLoopError::store(format!("invalid history message id: {e}"))
                    })
                })
                .collect::<Result<_>>()?;
            return Ok(MessageHistory {
                messages,
                sequences,
                source_sequence: response.source_sequence,
            });
        }
        let simple_window_query =
            query.filters.is_empty() && query.offset.is_none() && query.limit.is_some();
        let message_limit = if simple_window_query {
            query
                .limit
                .map(|limit| limit.clamp(0, i32::MAX as i64) as i32)
        } else {
            None
        };
        let (mut messages, total_count, source_sequence) = self
            .load_with_message_limit(query.session_id, message_limit, query.after_sequence)
            .await?;

        for filter in &query.filters {
            match filter {
                MessageFilter::TimeRange { from, to } => {
                    messages.retain(|m| {
                        let after_from = from.is_none_or(|t| m.created_at >= t);
                        let before_to = to.is_none_or(|t| m.created_at <= t);
                        after_from && before_to
                    });
                }
                MessageFilter::EventTypes(types) => {
                    messages.retain(|m| {
                        types.iter().any(|event_type| match event_type.as_str() {
                            "input.message" => m.role == RuntimeMessageRole::User,
                            "output.message.completed" => m.role == RuntimeMessageRole::Agent,
                            "tool.completed" => m.role == RuntimeMessageRole::ToolResult,
                            _ => false,
                        })
                    });
                }
                MessageFilter::Search(q) => {
                    let q_lower = q.to_lowercase();
                    messages
                        .retain(|m| m.content_to_llm_string().to_lowercase().contains(&q_lower));
                }
                MessageFilter::Custom(predicate) => {
                    messages.retain(|m| predicate(m));
                }
                MessageFilter::ToolName(_)
                | MessageFilter::ExcludeIds(_)
                | MessageFilter::IncludeIds(_) => {
                    return Err(AgentLoopError::store(format!(
                        "gRPC MessageRetriever does not support filter: {filter:?}"
                    )));
                }
            }
        }

        if simple_window_query {
            query.apply_window_bounds(&mut messages);
            query.prepend_excluded_notice(&mut messages, total_count);
        } else {
            query.apply_windowing(&mut messages);
        }

        if query.has_injections() {
            query.apply_injections(&mut messages);
        }

        Ok(MessageHistory::new(messages, source_sequence))
    }
}
