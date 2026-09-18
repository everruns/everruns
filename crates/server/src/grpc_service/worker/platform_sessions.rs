//! Platform session management and messaging.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_invoke_scheduled_app_channel(
        &self,
        request: Request<InvokeScheduledAppChannelRequest>,
    ) -> Result<Response<InvokeScheduledAppChannelResponse>, Status> {
        let req = request.into_inner();
        let runner = self
            .runner
            .clone()
            .ok_or_else(|| Status::unavailable("Agent runner not available"))?;
        let message_service = crate::domains::messages::MessageService::new(
            self.db.clone(),
            runner,
            false,
            self.event_service.event_delivery().clone(),
        );

        let result = crate::domains::apps::invoke_scheduled_app_channel(
            &self.db,
            self.encryption.as_ref(),
            &self.session_service,
            &message_service,
            req.org_id,
            &req.app_id,
            &req.channel_id,
        )
        .await
        .map_err(command_error_to_status)?;

        Ok(Response::new(InvokeScheduledAppChannelResponse {
            session_id: result.session_id.to_string(),
            created_session: result.created_session,
        }))
    }

    pub(crate) async fn handle_invoke_agent_trigger(
        &self,
        request: Request<InvokeAgentTriggerRequest>,
    ) -> Result<Response<InvokeAgentTriggerResponse>, Status> {
        let req = request.into_inner();
        let runner = self
            .runner
            .clone()
            .ok_or_else(|| Status::unavailable("Agent runner not available"))?;
        let message_service = crate::domains::messages::MessageService::new(
            self.db.clone(),
            runner,
            false,
            self.event_service.event_delivery().clone(),
        );

        let result = crate::domains::agent_triggers::invoke_agent_trigger(
            &self.db,
            &self.session_service,
            &message_service,
            req.org_id,
            &req.agent_id,
            &req.trigger_id,
        )
        .await
        .map_err(command_error_to_status)?;

        Ok(Response::new(InvokeAgentTriggerResponse {
            session_id: result.session_id.to_string(),
            created_session: result.created_session,
        }))
    }
}
