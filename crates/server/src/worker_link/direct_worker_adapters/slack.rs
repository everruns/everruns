//! Construct the session-bound Slack action service with host-owned egress.
use super::DirectWorkerAdapters;
use everruns_contracts::slack_action::SlackActionInvoker;
use everruns_contracts::typed_id::SessionId;
use std::sync::Arc;

impl DirectWorkerAdapters {
    pub(super) fn slack_invoker_for(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Arc<dyn SlackActionInvoker> {
        crate::channels::slack::actions::in_process_invoker(
            &self.db,
            self.encryption.as_ref(),
            self.egress_service.clone(),
            org_id,
            session_id,
        )
    }
}
