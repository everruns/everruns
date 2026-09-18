//! Session schedules.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use super::support::*;
use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_create_session_schedule(
        &self,
        request: Request<CreateSessionScheduleRequest>,
    ) -> Result<Response<CreateSessionScheduleResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let scheduled_at = req
            .scheduled_at
            .as_ref()
            .map(everruns_internal_protocol::proto_timestamp_to_datetime);

        let schedule = store
            .create_schedule_enforcing_limits(
                session_id.into(),
                req.description,
                req.cron_expression,
                scheduled_at,
                req.timezone,
            )
            .await
            .map_err(|e| match e {
                everruns_core::session_schedule::ScheduleLimitError::Rejected(msg) => {
                    Status::resource_exhausted(msg)
                }
                everruns_core::session_schedule::ScheduleLimitError::Store(err) => {
                    tracing::error!("Failed to create schedule: {}", err);
                    internal_status("Failed to create schedule", err)
                }
            })?;

        Ok(Response::new(CreateSessionScheduleResponse {
            schedule: Some(session_schedule_to_proto(&schedule)),
        }))
    }

    pub(crate) async fn handle_cancel_session_schedule(
        &self,
        request: Request<CancelSessionScheduleRequest>,
    ) -> Result<Response<CancelSessionScheduleResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let schedule_id = parse_uuid(req.schedule_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let schedule = store
            .cancel_schedule(
                session_id.into(),
                everruns_provider::typed_id::ScheduleId::from_uuid(schedule_id),
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to cancel schedule: {}", e);
                internal_status("Failed to cancel schedule", e)
            })?;

        Ok(Response::new(CancelSessionScheduleResponse {
            schedule: Some(session_schedule_to_proto(&schedule)),
        }))
    }

    pub(crate) async fn handle_list_session_schedules(
        &self,
        request: Request<ListSessionSchedulesRequest>,
    ) -> Result<Response<ListSessionSchedulesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let schedules = store.list_schedules(session_id.into()).await.map_err(|e| {
            tracing::error!("Failed to list schedules: {}", e);
            internal_status("Failed to list schedules", e)
        })?;

        let proto_schedules = schedules.iter().map(session_schedule_to_proto).collect();

        Ok(Response::new(ListSessionSchedulesResponse {
            schedules: proto_schedules,
        }))
    }

    pub(crate) async fn handle_count_active_session_schedules(
        &self,
        request: Request<CountActiveSessionSchedulesRequest>,
    ) -> Result<Response<CountActiveSessionSchedulesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.schedule_store(req.org_id)?;

        let count = store
            .count_active_schedules(session_id.into())
            .await
            .map_err(|e| {
                tracing::error!("Failed to count active schedules: {}", e);
                internal_status("Failed to count active schedules", e)
            })?;

        Ok(Response::new(CountActiveSessionSchedulesResponse { count }))
    }

    pub(crate) async fn handle_count_active_org_schedules(
        &self,
        request: Request<CountActiveOrgSchedulesRequest>,
    ) -> Result<Response<CountActiveOrgSchedulesResponse>, Status> {
        let req = request.into_inner();
        let store = self.schedule_store(req.org_id)?;

        let count = store.count_active_org_schedules().await.map_err(|e| {
            tracing::error!("Failed to count active org schedules: {}", e);
            internal_status("Failed to count active org schedules", e)
        })?;

        Ok(Response::new(CountActiveOrgSchedulesResponse { count }))
    }
}
