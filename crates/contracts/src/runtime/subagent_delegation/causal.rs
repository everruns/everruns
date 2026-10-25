//! Carry the creating turn across asynchronous child delivery without ambient state.
use super::*;
use crate::runtime::causal::CausalContext;
use std::sync::Arc;

pub(crate) struct CausalDelegate {
    pub inner: Arc<dyn SubagentSessionDelegate>,
    pub causes: CausalContext,
}

#[async_trait]
impl SubagentSessionDelegate for CausalDelegate {
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<AgentDefinition>> {
        self.inner.get_agent_by_id(id).await
    }
    async fn get_harness(&self, id: HarnessId) -> Result<Option<HarnessDefinition>> {
        self.inner.get_harness(id).await
    }
    async fn add_agent_session_participant(
        &self,
        session: SessionId,
        agent: AgentId,
        reason: &str,
    ) -> Result<SessionParticipantId> {
        self.inner
            .add_agent_session_participant(session, agent, reason)
            .await
    }
    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
        self.inner.create_session_with_options(request).await
    }
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
        self.inner.get_session_by_id(id).await
    }
    async fn send_message(&self, session: SessionId, content: &str) -> Result<()> {
        self.inner
            .send_message_with_causal_context(session, content, &self.causes)
            .await
    }
    async fn send_message_with_causal_context(
        &self,
        session: SessionId,
        content: &str,
        causes: &CausalContext,
    ) -> Result<()> {
        self.inner
            .send_message_with_causal_context(session, content, causes)
            .await
    }
    async fn get_messages(
        &self,
        session: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        self.inner.get_messages(session, limit).await
    }
    async fn wait_for_idle(&self, session: SessionId, timeout: Option<u64>) -> Result<String> {
        self.inner.wait_for_idle(session, timeout).await
    }
    async fn get_agent_harness_id(&self, id: AgentId) -> Result<Option<HarnessId>> {
        self.inner.get_agent_harness_id(id).await
    }
    async fn set_session_archived(
        &self,
        session: SessionId,
        archived: bool,
        reason: &str,
    ) -> Result<()> {
        self.inner
            .set_session_archived(session, archived, reason)
            .await
    }
}

impl crate::runtime::tool_context::ToolContext {
    pub(crate) fn bind_causal_delegate(&mut self, causes: CausalContext) {
        if !causes.is_empty()
            && let Some(inner) = self.subagent_delegate.take()
        {
            self.subagent_delegate = Some(Arc::new(CausalDelegate { inner, causes }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::typed_id::{EventId, MessageId, TurnId};
    use crate::runtime::{
        event_emitter::EventEmitter,
        events::{Event, EventContext, EventRequest},
        tool_context::ToolContext,
    };
    struct Emitter(CausalContext);
    #[async_trait]
    impl EventEmitter for Emitter {
        fn turn_causal_context(&self, _: &EventContext) -> CausalContext {
            self.0.clone()
        }
        async fn emit(&self, request: EventRequest) -> Result<Event> {
            Ok(request.into_event(EventId::new(), 1))
        }
    }

    #[tokio::test]
    async fn causal_delegate_survives_detached_delivery_and_absent_turn_binding() {
        let target = Arc::new(super::super::tests::MockSubagentDelegate::new());
        let mut context = ToolContext::new(SessionId::new()).with_subagent_delegate(target.clone());
        context.bind_to_turn(EventContext::empty());
        assert!(context.subagent_delegate.is_some());
        let causes = CausalContext::from_traceparents([
            "00-1234567890abcdef1234567890abcdef-1234567890abcdef-00",
        ]);
        context.event_emitter = Some(Arc::new(Emitter(causes.clone())));
        context.bind_to_turn(EventContext::turn(TurnId::new(), MessageId::new()));
        let delegate = context.subagent_delegate.take().unwrap();
        tokio::spawn(async move {
            delegate
                .send_message(SessionId::new(), "work")
                .await
                .unwrap();
        })
        .await
        .unwrap();
        assert_eq!(*target.causal_messages.lock().unwrap(), [causes]);
        assert_eq!(target.sent_messages.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn causal_delegate_restores_persisted_task_causes_without_an_observer() {
        let target = Arc::new(super::super::tests::MockSubagentDelegate::new());
        let mut context = ToolContext::new(SessionId::new()).with_subagent_delegate(target.clone());
        let causes = CausalContext::from_traceparents([
            "00-1234567890abcdef1234567890abcdef-1234567890abcdef-00",
        ]);
        context.bind_to_turn_with_causal_context(EventContext::empty(), causes.clone());
        context
            .subagent_delegate
            .unwrap()
            .send_message(SessionId::new(), "reattached")
            .await
            .unwrap();
        assert_eq!(*target.causal_messages.lock().unwrap(), [causes]);
    }
}
