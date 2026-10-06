// `WorkerService` trait implementation for the gRPC service.
//
// Delegation only. A trait impl cannot be split across modules, so every
// handler body lives in `super::worker::<domain>` as an inherent method and
// `delegate!` below generates the RPC that forwards to it. Keep it that way: a
// body added here is a body nobody will find again.

use super::*;

/// Generates the `WorkerService` impl. Each `rpc => handler(Req) -> Resp;`
/// line becomes `async fn rpc(&self, Request<Req>) -> Result<Response<Resp>,
/// Status>` forwarding to `self.handler`. (`macro_rules!` cannot build the
/// `handle_` identifier itself, so it is spelled out.)
macro_rules! delegate {
    ($($rpc:ident => $handler:ident($req:ty) -> $resp:ty;)*) => {
        #[tonic::async_trait]
        impl WorkerService for WorkerServiceImpl {
            type SubscribeTaskNotificationsStream =
                super::worker::support::TaskNotificationStream;

            $(
                async fn $rpc(
                    &self,
                    request: Request<$req>,
                ) -> Result<Response<$resp>, Status> {
                    self.$handler(request).await
                }
            )*
        }
    };
}

delegate! {
    // Turn context, message load/append, journal, compaction checkpoints.
    get_turn_context => handle_get_turn_context(GetTurnContextRequest) -> GetTurnContextResponse;
    get_message => handle_get_message(GetMessageRequest) -> GetMessageResponse;
    load_messages => handle_load_messages(LoadMessagesRequest) -> LoadMessagesResponse;
    native_async_journal => handle_native_async_journal(proto::NativeAsyncJournalRequest)
        -> proto::NativeAsyncJournalResponse;
    agents_api_journal => handle_agents_api_journal(proto::AgentsApiJournalRequest)
        -> proto::AgentsApiJournalResponse;
    get_compaction_checkpoint => handle_get_compaction_checkpoint(proto::GetCompactionCheckpointRequest)
        -> proto::GetCompactionCheckpointResponse;
    install_compaction_checkpoint => handle_install_compaction_checkpoint(proto::InstallCompactionCheckpointRequest)
        -> proto::InstallCompactionCheckpointResponse;
    get_partial_stream => handle_get_partial_stream(proto::GetPartialStreamRequest)
        -> proto::GetPartialStreamResponse;
    sandbox_persistence => handle_sandbox_persistence(proto::SandboxPersistenceRequest)
        -> proto::SandboxPersistenceResponse;
    add_message => handle_add_message(AddMessageRequest) -> AddMessageResponse;

    // Event emission and exec commits.
    emit_event_stream => handle_emit_event_stream(Streaming<EmitEventRequest>)
        -> EmitEventStreamResponse;
    emit_event => handle_emit_event(EmitEventRequest) -> EmitEventResponse;
    commit_exec => handle_commit_exec(CommitExecRequest) -> CommitExecResponse;

    // Agent, harness, session lookup and mutation, model resolution.
    get_agent => handle_get_agent(GetAgentRequest) -> GetAgentResponse;
    get_harness => handle_get_harness(GetHarnessRequest) -> GetHarnessResponse;
    get_session => handle_get_session(GetSessionRequest) -> GetSessionResponse;
    set_session_status => handle_set_session_status(SetSessionStatusRequest)
        -> SetSessionStatusResponse;
    set_session_title => handle_set_session_title(SetSessionTitleRequest)
        -> SetSessionTitleResponse;
    get_resolved_model => handle_get_resolved_model(GetResolvedModelRequest)
        -> GetResolvedModelResponse;
    get_default_model => handle_get_default_model(GetDefaultModelRequest)
        -> GetDefaultModelResponse;

    // Durable workflows, tasks, and workers.
    create_durable_workflow => handle_create_durable_workflow(CreateDurableWorkflowRequest)
        -> CreateDurableWorkflowResponse;
    get_durable_workflow_status => handle_get_durable_workflow_status(GetDurableWorkflowStatusRequest)
        -> GetDurableWorkflowStatusResponse;
    update_durable_workflow_status => handle_update_durable_workflow_status(UpdateDurableWorkflowStatusRequest)
        -> UpdateDurableWorkflowStatusResponse;
    enqueue_durable_task => handle_enqueue_durable_task(EnqueueDurableTaskRequest)
        -> EnqueueDurableTaskResponse;
    claim_durable_tasks => handle_claim_durable_tasks(ClaimDurableTasksRequest)
        -> ClaimDurableTasksResponse;
    complete_durable_task => handle_complete_durable_task(CompleteDurableTaskRequest)
        -> CompleteDurableTaskResponse;
    fail_durable_task => handle_fail_durable_task(FailDurableTaskRequest)
        -> FailDurableTaskResponse;
    heartbeat_durable_task => handle_heartbeat_durable_task(HeartbeatDurableTaskRequest)
        -> HeartbeatDurableTaskResponse;
    count_active_durable_workflows => handle_count_active_durable_workflows(CountActiveDurableWorkflowsRequest)
        -> CountActiveDurableWorkflowsResponse;
    send_durable_workflow_signal => handle_send_durable_workflow_signal(SendDurableWorkflowSignalRequest)
        -> SendDurableWorkflowSignalResponse;
    get_and_consume_durable_workflow_signals => handle_get_and_consume_durable_workflow_signals(GetAndConsumeDurableWorkflowSignalsRequest)
        -> GetAndConsumeDurableWorkflowSignalsResponse;
    register_durable_worker => handle_register_durable_worker(RegisterDurableWorkerRequest)
        -> RegisterDurableWorkerResponse;
    heartbeat_durable_worker => handle_heartbeat_durable_worker(HeartbeatDurableWorkerRequest)
        -> HeartbeatDurableWorkerResponse;
    drain_durable_worker => handle_drain_durable_worker(DrainDurableWorkerRequest)
        -> DrainDurableWorkerResponse;
    deregister_durable_worker => handle_deregister_durable_worker(DeregisterDurableWorkerRequest)
        -> DeregisterDurableWorkerResponse;

    // Circuit breaker state.
    check_circuit_breaker => handle_check_circuit_breaker(CheckCircuitBreakerRequest)
        -> CheckCircuitBreakerResponse;
    record_circuit_breaker_success => handle_record_circuit_breaker_success(RecordCircuitBreakerSuccessRequest)
        -> RecordCircuitBreakerSuccessResponse;
    record_circuit_breaker_failure => handle_record_circuit_breaker_failure(RecordCircuitBreakerFailureRequest)
        -> RecordCircuitBreakerFailureResponse;

    // Task notification stream.
    subscribe_task_notifications => handle_subscribe_task_notifications(SubscribeTaskNotificationsRequest)
        -> Self::SubscribeTaskNotificationsStream;

    // Image and file artifact resolution.
    resolve_image => handle_resolve_image(ResolveImageRequest) -> ResolveImageResponse;
    resolve_images => handle_resolve_images(ResolveImagesRequest) -> ResolveImagesResponse;
    resolve_files => handle_resolve_files(ResolveFilesRequest) -> ResolveFilesResponse;
    create_image_artifact => handle_create_image_artifact(CreateImageArtifactRequest)
        -> CreateImageArtifactResponse;
    get_image_artifact => handle_get_image_artifact(GetImageArtifactRequest)
        -> GetImageArtifactResponse;
    get_image_artifact_info => handle_get_image_artifact_info(GetImageArtifactInfoRequest)
        -> GetImageArtifactInfoResponse;

    // Provider credentials and MCP server resolution.
    get_default_provider_credentials => handle_get_default_provider_credentials(GetDefaultProviderCredentialsRequest)
        -> GetDefaultProviderCredentialsResponse;
    get_mcp_server_by_prefix => handle_get_mcp_server_by_prefix(GetMcpServerByPrefixRequest)
        -> GetMcpServerByPrefixResponse;

    // Session key/value storage and secrets.
    session_storage_set_value => handle_session_storage_set_value(SessionStorageSetValueRequest)
        -> SessionStorageSetValueResponse;
    session_storage_get_value => handle_session_storage_get_value(SessionStorageGetValueRequest)
        -> SessionStorageGetValueResponse;
    session_storage_delete_value => handle_session_storage_delete_value(SessionStorageDeleteValueRequest)
        -> SessionStorageDeleteValueResponse;
    session_storage_take_value => handle_session_storage_take_value(SessionStorageTakeValueRequest)
        -> SessionStorageTakeValueResponse;
    session_storage_list_keys => handle_session_storage_list_keys(SessionStorageListKeysRequest)
        -> SessionStorageListKeysResponse;
    session_storage_set_secret => handle_session_storage_set_secret(SessionStorageSetSecretRequest)
        -> SessionStorageSetSecretResponse;
    session_storage_get_secret => handle_session_storage_get_secret(SessionStorageGetSecretRequest)
        -> SessionStorageGetSecretResponse;
    session_storage_delete_secret => handle_session_storage_delete_secret(SessionStorageDeleteSecretRequest)
        -> SessionStorageDeleteSecretResponse;
    session_storage_list_secrets => handle_session_storage_list_secrets(SessionStorageListSecretsRequest)
        -> SessionStorageListSecretsResponse;

    // User connection tokens.
    get_connection_token => handle_get_connection_token(GetConnectionTokenRequest)
        -> GetConnectionTokenResponse;
    get_mcp_connection_token => handle_get_mcp_connection_token(GetMcpConnectionTokenRequest)
        -> GetConnectionTokenResponse;
    invalidate_mcp_connection => handle_invalidate_mcp_connection(InvalidateMcpConnectionRequest)
        -> InvalidateMcpConnectionResponse;
    get_connection_user => handle_get_connection_user(GetConnectionUserRequest)
        -> GetConnectionUserResponse;
    get_connection_token_for_user => handle_get_connection_token_for_user(GetConnectionTokenForUserRequest)
        -> GetConnectionTokenForUserResponse;

    // Leased resource lifecycle.
    upsert_leased_resource => handle_upsert_leased_resource(UpsertLeasedResourceRequest)
        -> UpsertLeasedResourceResponse;
    release_leased_resource => handle_release_leased_resource(ReleaseLeasedResourceRequest)
        -> ReleaseLeasedResourceResponse;
    list_session_leased_resources => handle_list_session_leased_resources(ListSessionLeasedResourcesRequest)
        -> ListSessionLeasedResourcesResponse;
    claim_due_leased_resources => handle_claim_due_leased_resources(ClaimDueLeasedResourcesRequest)
        -> ClaimDueLeasedResourcesResponse;
    mark_leased_resource_released => handle_mark_leased_resource_released(MarkLeasedResourceReleasedRequest)
        -> MarkLeasedResourceReleasedResponse;
    mark_leased_resource_cleanup_failed => handle_mark_leased_resource_cleanup_failed(MarkLeasedResourceCleanupFailedRequest)
        -> MarkLeasedResourceCleanupFailedResponse;

    // Session resource registry.
    register_session_resource => handle_register_session_resource(RegisterSessionResourceRequest)
        -> RegisterSessionResourceResponse;
    update_session_resource_status => handle_update_session_resource_status(UpdateSessionResourceStatusRequest)
        -> UpdateSessionResourceStatusResponse;
    list_session_resources => handle_list_session_resources(ListSessionResourcesRequest)
        -> ListSessionResourcesResponse;
    deregister_session_resource => handle_deregister_session_resource(DeregisterSessionResourceRequest)
        -> DeregisterSessionResourceResponse;

    // Session task lifecycle and task messages.
    create_session_task => handle_create_session_task(CreateSessionTaskRequest)
        -> SessionTaskResponse;
    update_session_task => handle_update_session_task(UpdateSessionTaskRequest)
        -> OptionalSessionTaskResponse;
    get_session_task => handle_get_session_task(GetSessionTaskRequest)
        -> OptionalSessionTaskResponse;
    list_session_tasks => handle_list_session_tasks(ListSessionTasksRequest)
        -> ListSessionTasksResponse;
    request_cancel_session_task => handle_request_cancel_session_task(RequestCancelSessionTaskRequest)
        -> OptionalSessionTaskResponse;
    record_session_task_message => handle_record_session_task_message(RecordSessionTaskMessageRequest)
        -> SessionTaskMessageResponse;
    list_session_task_messages => handle_list_session_task_messages(ListSessionTaskMessagesRequest)
        -> ListSessionTaskMessagesResponse;
    list_orphaned_session_tasks => handle_list_orphaned_session_tasks(ListOrphanedSessionTasksRequest)
        -> ListOrphanedSessionTasksResponse;
    prune_terminal_session_tasks => handle_prune_terminal_session_tasks(PruneTerminalSessionTasksRequest)
        -> PruneTerminalSessionTasksResponse;

    // Session schedules.
    create_session_schedule => handle_create_session_schedule(CreateSessionScheduleRequest)
        -> CreateSessionScheduleResponse;
    cancel_session_schedule => handle_cancel_session_schedule(CancelSessionScheduleRequest)
        -> CancelSessionScheduleResponse;
    list_session_schedules => handle_list_session_schedules(ListSessionSchedulesRequest)
        -> ListSessionSchedulesResponse;
    count_active_session_schedules => handle_count_active_session_schedules(CountActiveSessionSchedulesRequest)
        -> CountActiveSessionSchedulesResponse;
    count_active_org_schedules => handle_count_active_org_schedules(CountActiveOrgSchedulesRequest)
        -> CountActiveOrgSchedulesResponse;

    // Session SQL databases.

    session_sql_db_execute => handle_session_sql_db_execute(SessionSqlDbExecuteRequest)
        -> SessionSqlDbExecuteResponse;
    session_sql_db_query => handle_session_sql_db_query(SessionSqlDbQueryRequest)
        -> SessionSqlDbQueryResponse;

    // Generic domain command transport.
    execute_command => handle_execute_command(ExecuteCommandRequest) -> ExecuteCommandResponse;
    list_commands => handle_list_commands(ListCommandsRequest) -> ListCommandsResponse;
    invoke_platform_command_surface => handle_invoke_platform_command_surface(InvokePlatformCommandSurfaceRequest)
        -> InvokePlatformCommandSurfaceResponse;

    // Platform harness management.

    // Platform agent management.

    // Platform session management and messaging.

    invoke_scheduled_app_channel => handle_invoke_scheduled_app_channel(InvokeScheduledAppChannelRequest)
        -> InvokeScheduledAppChannelResponse;
    invoke_agent_trigger => handle_invoke_agent_trigger(InvokeAgentTriggerRequest)
        -> InvokeAgentTriggerResponse;

    // Platform capability and base-URL lookup.

    // Budgets, rate limits, payments, and session authorization.
    check_budgets_for_session => handle_check_budgets_for_session(CheckBudgetsForSessionRequest)
        -> CheckBudgetsForSessionResponse;
    check_outbound_tool_rate_limit => handle_check_outbound_tool_rate_limit(CheckOutboundToolRateLimitRequest)
        -> CheckOutboundToolRateLimitResponse;
    invoke_slack_action => handle_invoke_slack_action(InvokeSlackActionRequest)
        -> InvokeSlackActionResponse;
    execute_machine_payment => handle_execute_machine_payment(ExecuteMachinePaymentRequest)
        -> ExecuteMachinePaymentResponse;
    authorize_session_creation => handle_authorize_session_creation(AuthorizeSessionCreationRequest)
        -> AuthorizeSessionCreationResponse;
}
