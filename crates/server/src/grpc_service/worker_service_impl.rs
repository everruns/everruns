// `WorkerService` trait implementation for the gRPC service.
//
// Delegation only. A trait impl cannot be split across modules, and this one has
// 119 methods, so every handler body lives in `super::worker::<domain>` as an
// inherent method and each RPC below forwards to it. Keep it that way: a body
// added here is a body nobody will find again.

use super::*;

#[tonic::async_trait]
impl WorkerService for WorkerServiceImpl {
    type SubscribeTaskNotificationsStream = super::worker::support::TaskNotificationStream;

    // Turn context, message load/append, journal, compaction checkpoints.
    async fn get_turn_context(
        &self,
        request: Request<GetTurnContextRequest>,
    ) -> Result<Response<GetTurnContextResponse>, Status> {
        self.handle_get_turn_context(request).await
    }

    async fn get_message(
        &self,
        request: Request<GetMessageRequest>,
    ) -> Result<Response<GetMessageResponse>, Status> {
        self.handle_get_message(request).await
    }

    async fn load_messages(
        &self,
        request: Request<LoadMessagesRequest>,
    ) -> Result<Response<LoadMessagesResponse>, Status> {
        self.handle_load_messages(request).await
    }

    async fn native_async_journal(
        &self,
        request: Request<proto::NativeAsyncJournalRequest>,
    ) -> Result<Response<proto::NativeAsyncJournalResponse>, Status> {
        self.handle_native_async_journal(request).await
    }

    async fn get_compaction_checkpoint(
        &self,
        request: Request<proto::GetCompactionCheckpointRequest>,
    ) -> Result<Response<proto::GetCompactionCheckpointResponse>, Status> {
        self.handle_get_compaction_checkpoint(request).await
    }

    async fn install_compaction_checkpoint(
        &self,
        request: Request<proto::InstallCompactionCheckpointRequest>,
    ) -> Result<Response<proto::InstallCompactionCheckpointResponse>, Status> {
        self.handle_install_compaction_checkpoint(request).await
    }

    async fn add_message(
        &self,
        request: Request<AddMessageRequest>,
    ) -> Result<Response<AddMessageResponse>, Status> {
        self.handle_add_message(request).await
    }

    // Event emission and exec commits.
    async fn emit_event_stream(
        &self,
        request: Request<Streaming<EmitEventRequest>>,
    ) -> Result<Response<EmitEventStreamResponse>, Status> {
        self.handle_emit_event_stream(request).await
    }

    async fn emit_event(
        &self,
        request: Request<EmitEventRequest>,
    ) -> Result<Response<EmitEventResponse>, Status> {
        self.handle_emit_event(request).await
    }

    async fn commit_exec(
        &self,
        _request: Request<CommitExecRequest>,
    ) -> Result<Response<CommitExecResponse>, Status> {
        self.handle_commit_exec(_request).await
    }

    // Agent, harness, session lookup and mutation, model resolution.
    async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<GetAgentResponse>, Status> {
        self.handle_get_agent(request).await
    }

    async fn get_harness(
        &self,
        request: Request<GetHarnessRequest>,
    ) -> Result<Response<GetHarnessResponse>, Status> {
        self.handle_get_harness(request).await
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        self.handle_get_session(request).await
    }

    async fn set_session_status(
        &self,
        request: Request<SetSessionStatusRequest>,
    ) -> Result<Response<SetSessionStatusResponse>, Status> {
        self.handle_set_session_status(request).await
    }

    async fn set_session_title(
        &self,
        request: Request<SetSessionTitleRequest>,
    ) -> Result<Response<SetSessionTitleResponse>, Status> {
        self.handle_set_session_title(request).await
    }

    async fn get_resolved_model(
        &self,
        request: Request<GetResolvedModelRequest>,
    ) -> Result<Response<GetResolvedModelResponse>, Status> {
        self.handle_get_resolved_model(request).await
    }

    async fn get_default_model(
        &self,
        request: Request<GetDefaultModelRequest>,
    ) -> Result<Response<GetDefaultModelResponse>, Status> {
        self.handle_get_default_model(request).await
    }

    // Durable workflows, tasks, and workers.
    async fn create_durable_workflow(
        &self,
        request: Request<CreateDurableWorkflowRequest>,
    ) -> Result<Response<CreateDurableWorkflowResponse>, Status> {
        self.handle_create_durable_workflow(request).await
    }

    async fn get_durable_workflow_status(
        &self,
        request: Request<GetDurableWorkflowStatusRequest>,
    ) -> Result<Response<GetDurableWorkflowStatusResponse>, Status> {
        self.handle_get_durable_workflow_status(request).await
    }

    async fn update_durable_workflow_status(
        &self,
        request: Request<UpdateDurableWorkflowStatusRequest>,
    ) -> Result<Response<UpdateDurableWorkflowStatusResponse>, Status> {
        self.handle_update_durable_workflow_status(request).await
    }

    async fn enqueue_durable_task(
        &self,
        request: Request<EnqueueDurableTaskRequest>,
    ) -> Result<Response<EnqueueDurableTaskResponse>, Status> {
        self.handle_enqueue_durable_task(request).await
    }

    async fn claim_durable_tasks(
        &self,
        request: Request<ClaimDurableTasksRequest>,
    ) -> Result<Response<ClaimDurableTasksResponse>, Status> {
        self.handle_claim_durable_tasks(request).await
    }

    async fn complete_durable_task(
        &self,
        request: Request<CompleteDurableTaskRequest>,
    ) -> Result<Response<CompleteDurableTaskResponse>, Status> {
        self.handle_complete_durable_task(request).await
    }

    async fn fail_durable_task(
        &self,
        request: Request<FailDurableTaskRequest>,
    ) -> Result<Response<FailDurableTaskResponse>, Status> {
        self.handle_fail_durable_task(request).await
    }

    async fn heartbeat_durable_task(
        &self,
        request: Request<HeartbeatDurableTaskRequest>,
    ) -> Result<Response<HeartbeatDurableTaskResponse>, Status> {
        self.handle_heartbeat_durable_task(request).await
    }

    async fn count_active_durable_workflows(
        &self,
        _request: Request<CountActiveDurableWorkflowsRequest>,
    ) -> Result<Response<CountActiveDurableWorkflowsResponse>, Status> {
        self.handle_count_active_durable_workflows(_request).await
    }

    async fn send_durable_workflow_signal(
        &self,
        request: Request<SendDurableWorkflowSignalRequest>,
    ) -> Result<Response<SendDurableWorkflowSignalResponse>, Status> {
        self.handle_send_durable_workflow_signal(request).await
    }

    async fn get_and_consume_durable_workflow_signals(
        &self,
        request: Request<GetAndConsumeDurableWorkflowSignalsRequest>,
    ) -> Result<Response<GetAndConsumeDurableWorkflowSignalsResponse>, Status> {
        self.handle_get_and_consume_durable_workflow_signals(request)
            .await
    }

    async fn register_durable_worker(
        &self,
        request: Request<RegisterDurableWorkerRequest>,
    ) -> Result<Response<RegisterDurableWorkerResponse>, Status> {
        self.handle_register_durable_worker(request).await
    }

    async fn heartbeat_durable_worker(
        &self,
        request: Request<HeartbeatDurableWorkerRequest>,
    ) -> Result<Response<HeartbeatDurableWorkerResponse>, Status> {
        self.handle_heartbeat_durable_worker(request).await
    }

    async fn deregister_durable_worker(
        &self,
        request: Request<DeregisterDurableWorkerRequest>,
    ) -> Result<Response<DeregisterDurableWorkerResponse>, Status> {
        self.handle_deregister_durable_worker(request).await
    }

    // Circuit breaker state.
    async fn check_circuit_breaker(
        &self,
        request: Request<CheckCircuitBreakerRequest>,
    ) -> Result<Response<CheckCircuitBreakerResponse>, Status> {
        self.handle_check_circuit_breaker(request).await
    }

    async fn record_circuit_breaker_success(
        &self,
        request: Request<RecordCircuitBreakerSuccessRequest>,
    ) -> Result<Response<RecordCircuitBreakerSuccessResponse>, Status> {
        self.handle_record_circuit_breaker_success(request).await
    }

    async fn record_circuit_breaker_failure(
        &self,
        request: Request<RecordCircuitBreakerFailureRequest>,
    ) -> Result<Response<RecordCircuitBreakerFailureResponse>, Status> {
        self.handle_record_circuit_breaker_failure(request).await
    }

    // Task notification stream.
    async fn subscribe_task_notifications(
        &self,
        request: Request<SubscribeTaskNotificationsRequest>,
    ) -> Result<Response<Self::SubscribeTaskNotificationsStream>, Status> {
        self.handle_subscribe_task_notifications(request).await
    }

    // Image and file artifact resolution.
    async fn resolve_image(
        &self,
        request: Request<ResolveImageRequest>,
    ) -> Result<Response<ResolveImageResponse>, Status> {
        self.handle_resolve_image(request).await
    }

    async fn resolve_images(
        &self,
        request: Request<ResolveImagesRequest>,
    ) -> Result<Response<ResolveImagesResponse>, Status> {
        self.handle_resolve_images(request).await
    }

    async fn resolve_files(
        &self,
        request: Request<ResolveFilesRequest>,
    ) -> Result<Response<ResolveFilesResponse>, Status> {
        self.handle_resolve_files(request).await
    }

    async fn create_image_artifact(
        &self,
        request: Request<CreateImageArtifactRequest>,
    ) -> Result<Response<CreateImageArtifactResponse>, Status> {
        self.handle_create_image_artifact(request).await
    }

    async fn get_image_artifact(
        &self,
        request: Request<GetImageArtifactRequest>,
    ) -> Result<Response<GetImageArtifactResponse>, Status> {
        self.handle_get_image_artifact(request).await
    }

    async fn get_image_artifact_info(
        &self,
        request: Request<GetImageArtifactInfoRequest>,
    ) -> Result<Response<GetImageArtifactInfoResponse>, Status> {
        self.handle_get_image_artifact_info(request).await
    }

    // Provider credentials and MCP server resolution.
    async fn get_default_provider_credentials(
        &self,
        request: Request<GetDefaultProviderCredentialsRequest>,
    ) -> Result<Response<GetDefaultProviderCredentialsResponse>, Status> {
        self.handle_get_default_provider_credentials(request).await
    }

    async fn get_mcp_server_by_prefix(
        &self,
        request: Request<GetMcpServerByPrefixRequest>,
    ) -> Result<Response<GetMcpServerByPrefixResponse>, Status> {
        self.handle_get_mcp_server_by_prefix(request).await
    }

    // Session key/value storage and secrets.
    async fn session_storage_set_value(
        &self,
        request: Request<SessionStorageSetValueRequest>,
    ) -> Result<Response<SessionStorageSetValueResponse>, Status> {
        self.handle_session_storage_set_value(request).await
    }

    async fn session_storage_get_value(
        &self,
        request: Request<SessionStorageGetValueRequest>,
    ) -> Result<Response<SessionStorageGetValueResponse>, Status> {
        self.handle_session_storage_get_value(request).await
    }

    async fn session_storage_delete_value(
        &self,
        request: Request<SessionStorageDeleteValueRequest>,
    ) -> Result<Response<SessionStorageDeleteValueResponse>, Status> {
        self.handle_session_storage_delete_value(request).await
    }

    async fn session_storage_take_value(
        &self,
        request: Request<SessionStorageTakeValueRequest>,
    ) -> Result<Response<SessionStorageTakeValueResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.storage_store()?;

        let value = store
            .take_value(session_id.into(), &req.key)
            .await
            .map_err(|error| internal_status("Failed to take storage value", error))?;

        Ok(Response::new(SessionStorageTakeValueResponse { value }))
    }

    async fn session_storage_list_keys(
        &self,
        request: Request<SessionStorageListKeysRequest>,
    ) -> Result<Response<SessionStorageListKeysResponse>, Status> {
        self.handle_session_storage_list_keys(request).await
    }

    async fn session_storage_set_secret(
        &self,
        request: Request<SessionStorageSetSecretRequest>,
    ) -> Result<Response<SessionStorageSetSecretResponse>, Status> {
        self.handle_session_storage_set_secret(request).await
    }

    async fn session_storage_get_secret(
        &self,
        request: Request<SessionStorageGetSecretRequest>,
    ) -> Result<Response<SessionStorageGetSecretResponse>, Status> {
        self.handle_session_storage_get_secret(request).await
    }

    async fn session_storage_delete_secret(
        &self,
        request: Request<SessionStorageDeleteSecretRequest>,
    ) -> Result<Response<SessionStorageDeleteSecretResponse>, Status> {
        self.handle_session_storage_delete_secret(request).await
    }

    async fn session_storage_list_secrets(
        &self,
        request: Request<SessionStorageListSecretsRequest>,
    ) -> Result<Response<SessionStorageListSecretsResponse>, Status> {
        self.handle_session_storage_list_secrets(request).await
    }

    // User connection tokens.
    async fn get_connection_token(
        &self,
        request: Request<GetConnectionTokenRequest>,
    ) -> Result<Response<GetConnectionTokenResponse>, Status> {
        self.handle_get_connection_token(request).await
    }

    async fn get_mcp_connection_token(
        &self,
        request: Request<GetMcpConnectionTokenRequest>,
    ) -> Result<Response<GetConnectionTokenResponse>, Status> {
        self.handle_get_mcp_connection_token(request).await
    }
    async fn invalidate_mcp_connection(
        &self,
        request: Request<InvalidateMcpConnectionRequest>,
    ) -> Result<Response<InvalidateMcpConnectionResponse>, Status> {
        self.handle_invalidate_mcp_connection(request).await
    }

    async fn get_connection_user(
        &self,
        request: Request<GetConnectionUserRequest>,
    ) -> Result<Response<GetConnectionUserResponse>, Status> {
        self.handle_get_connection_user(request).await
    }

    async fn get_connection_token_for_user(
        &self,
        request: Request<GetConnectionTokenForUserRequest>,
    ) -> Result<Response<GetConnectionTokenForUserResponse>, Status> {
        self.handle_get_connection_token_for_user(request).await
    }

    // Leased resource lifecycle.
    async fn upsert_leased_resource(
        &self,
        request: Request<UpsertLeasedResourceRequest>,
    ) -> Result<Response<UpsertLeasedResourceResponse>, Status> {
        self.handle_upsert_leased_resource(request).await
    }

    async fn release_leased_resource(
        &self,
        request: Request<ReleaseLeasedResourceRequest>,
    ) -> Result<Response<ReleaseLeasedResourceResponse>, Status> {
        self.handle_release_leased_resource(request).await
    }

    async fn list_session_leased_resources(
        &self,
        request: Request<ListSessionLeasedResourcesRequest>,
    ) -> Result<Response<ListSessionLeasedResourcesResponse>, Status> {
        self.handle_list_session_leased_resources(request).await
    }

    async fn claim_due_leased_resources(
        &self,
        request: Request<ClaimDueLeasedResourcesRequest>,
    ) -> Result<Response<ClaimDueLeasedResourcesResponse>, Status> {
        self.handle_claim_due_leased_resources(request).await
    }

    async fn mark_leased_resource_released(
        &self,
        request: Request<MarkLeasedResourceReleasedRequest>,
    ) -> Result<Response<MarkLeasedResourceReleasedResponse>, Status> {
        self.handle_mark_leased_resource_released(request).await
    }

    async fn mark_leased_resource_cleanup_failed(
        &self,
        request: Request<MarkLeasedResourceCleanupFailedRequest>,
    ) -> Result<Response<MarkLeasedResourceCleanupFailedResponse>, Status> {
        self.handle_mark_leased_resource_cleanup_failed(request)
            .await
    }

    // Session resource registry.
    async fn register_session_resource(
        &self,
        request: Request<RegisterSessionResourceRequest>,
    ) -> Result<Response<RegisterSessionResourceResponse>, Status> {
        self.handle_register_session_resource(request).await
    }

    async fn update_session_resource_status(
        &self,
        request: Request<UpdateSessionResourceStatusRequest>,
    ) -> Result<Response<UpdateSessionResourceStatusResponse>, Status> {
        self.handle_update_session_resource_status(request).await
    }

    async fn list_session_resources(
        &self,
        request: Request<ListSessionResourcesRequest>,
    ) -> Result<Response<ListSessionResourcesResponse>, Status> {
        self.handle_list_session_resources(request).await
    }

    async fn deregister_session_resource(
        &self,
        request: Request<DeregisterSessionResourceRequest>,
    ) -> Result<Response<DeregisterSessionResourceResponse>, Status> {
        self.handle_deregister_session_resource(request).await
    }

    // Session task lifecycle and task messages.
    async fn create_session_task(
        &self,
        request: Request<CreateSessionTaskRequest>,
    ) -> Result<Response<SessionTaskResponse>, Status> {
        self.handle_create_session_task(request).await
    }

    async fn update_session_task(
        &self,
        request: Request<UpdateSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        self.handle_update_session_task(request).await
    }

    async fn get_session_task(
        &self,
        request: Request<GetSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        self.handle_get_session_task(request).await
    }

    async fn list_session_tasks(
        &self,
        request: Request<ListSessionTasksRequest>,
    ) -> Result<Response<ListSessionTasksResponse>, Status> {
        self.handle_list_session_tasks(request).await
    }

    async fn request_cancel_session_task(
        &self,
        request: Request<RequestCancelSessionTaskRequest>,
    ) -> Result<Response<OptionalSessionTaskResponse>, Status> {
        self.handle_request_cancel_session_task(request).await
    }

    async fn record_session_task_message(
        &self,
        request: Request<RecordSessionTaskMessageRequest>,
    ) -> Result<Response<SessionTaskMessageResponse>, Status> {
        self.handle_record_session_task_message(request).await
    }

    async fn list_session_task_messages(
        &self,
        request: Request<ListSessionTaskMessagesRequest>,
    ) -> Result<Response<ListSessionTaskMessagesResponse>, Status> {
        self.handle_list_session_task_messages(request).await
    }

    async fn list_orphaned_session_tasks(
        &self,
        request: Request<ListOrphanedSessionTasksRequest>,
    ) -> Result<Response<ListOrphanedSessionTasksResponse>, Status> {
        self.handle_list_orphaned_session_tasks(request).await
    }

    async fn prune_terminal_session_tasks(
        &self,
        request: Request<PruneTerminalSessionTasksRequest>,
    ) -> Result<Response<PruneTerminalSessionTasksResponse>, Status> {
        self.handle_prune_terminal_session_tasks(request).await
    }

    // Session schedules.
    async fn create_session_schedule(
        &self,
        request: Request<CreateSessionScheduleRequest>,
    ) -> Result<Response<CreateSessionScheduleResponse>, Status> {
        self.handle_create_session_schedule(request).await
    }

    async fn cancel_session_schedule(
        &self,
        request: Request<CancelSessionScheduleRequest>,
    ) -> Result<Response<CancelSessionScheduleResponse>, Status> {
        self.handle_cancel_session_schedule(request).await
    }

    async fn list_session_schedules(
        &self,
        request: Request<ListSessionSchedulesRequest>,
    ) -> Result<Response<ListSessionSchedulesResponse>, Status> {
        self.handle_list_session_schedules(request).await
    }

    async fn count_active_session_schedules(
        &self,
        request: Request<CountActiveSessionSchedulesRequest>,
    ) -> Result<Response<CountActiveSessionSchedulesResponse>, Status> {
        self.handle_count_active_session_schedules(request).await
    }

    async fn count_active_org_schedules(
        &self,
        request: Request<CountActiveOrgSchedulesRequest>,
    ) -> Result<Response<CountActiveOrgSchedulesResponse>, Status> {
        self.handle_count_active_org_schedules(request).await
    }

    // Session SQL databases.

    async fn session_sql_db_execute(
        &self,
        request: Request<SessionSqlDbExecuteRequest>,
    ) -> Result<Response<SessionSqlDbExecuteResponse>, Status> {
        self.handle_session_sql_db_execute(request).await
    }

    async fn session_sql_db_query(
        &self,
        request: Request<SessionSqlDbQueryRequest>,
    ) -> Result<Response<SessionSqlDbQueryResponse>, Status> {
        self.handle_session_sql_db_query(request).await
    }

    // Generic domain command transport.
    async fn execute_command(
        &self,
        request: Request<ExecuteCommandRequest>,
    ) -> Result<Response<ExecuteCommandResponse>, Status> {
        self.handle_execute_command(request).await
    }

    async fn list_commands(
        &self,
        _request: Request<ListCommandsRequest>,
    ) -> Result<Response<ListCommandsResponse>, Status> {
        self.handle_list_commands(_request).await
    }

    async fn invoke_platform_command_surface(
        &self,
        request: Request<InvokePlatformCommandSurfaceRequest>,
    ) -> Result<Response<InvokePlatformCommandSurfaceResponse>, Status> {
        self.handle_invoke_platform_command_surface(request).await
    }

    // Platform harness management.

    // Platform agent management.

    // Platform session management and messaging.

    async fn invoke_scheduled_app_channel(
        &self,
        request: Request<InvokeScheduledAppChannelRequest>,
    ) -> Result<Response<InvokeScheduledAppChannelResponse>, Status> {
        self.handle_invoke_scheduled_app_channel(request).await
    }

    async fn invoke_agent_trigger(
        &self,
        request: Request<InvokeAgentTriggerRequest>,
    ) -> Result<Response<InvokeAgentTriggerResponse>, Status> {
        self.handle_invoke_agent_trigger(request).await
    }

    // Platform capability and base-URL lookup.

    // Budgets, rate limits, payments, and session authorization.
    async fn check_budgets_for_session(
        &self,
        request: Request<CheckBudgetsForSessionRequest>,
    ) -> Result<Response<CheckBudgetsForSessionResponse>, Status> {
        self.handle_check_budgets_for_session(request).await
    }

    async fn check_outbound_tool_rate_limit(
        &self,
        request: Request<CheckOutboundToolRateLimitRequest>,
    ) -> Result<Response<CheckOutboundToolRateLimitResponse>, Status> {
        self.handle_check_outbound_tool_rate_limit(request).await
    }

    async fn invoke_slack_action(
        &self,
        request: Request<InvokeSlackActionRequest>,
    ) -> Result<Response<InvokeSlackActionResponse>, Status> {
        crate::slack_actions::serve_rpc(&self.db, self.encryption.as_ref(), request).await
    }

    async fn execute_machine_payment(
        &self,
        request: Request<ExecuteMachinePaymentRequest>,
    ) -> Result<Response<ExecuteMachinePaymentResponse>, Status> {
        self.handle_execute_machine_payment(request).await
    }

    async fn authorize_session_creation(
        &self,
        request: Request<AuthorizeSessionCreationRequest>,
    ) -> Result<Response<AuthorizeSessionCreationResponse>, Status> {
        self.handle_authorize_session_creation(request).await
    }
}
