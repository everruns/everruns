// Worker link and background loops started by the server (startup phase 7).
//
// Decision: production serves workers over gRPC; DEV_MODE runs one in-process
//   task worker over `DirectWorkerAdapters` instead. Both get the same
//   dependencies from `WorkerLinkDeps`, so a service wired into one is visible
//   in the other.

use crate::background::supervised_task::{RestartPolicy, TaskSupervisor};
use crate::domains::agent_channels::record::slack_provisioning::SlackAppProvisioner;
use crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry;
use crate::storage::{EncryptionService, StorageBackend};
use crate::worker_link::direct_worker_adapters::DirectWorkerAdapters;
use crate::worker_link::grpc_service;
use crate::{domains, services};
use anyhow::{Context, Result};
use everruns_core::host::HostComposition;
use everruns_core::permissions::PermissionResolver;
use everruns_durable::{PostgresWorkflowEventStore, WorkflowEventStore};
use everruns_worker::{TaskWorker, TaskWorkerConfig};
use std::sync::Arc;
use std::time::Duration;

/// Services a worker reaches through the server, whichever transport links them.
#[derive(Clone)]
pub(super) struct WorkerLinkDeps {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub event_service: Arc<services::EventService>,
    pub runner: Arc<dyn everruns_core::host::TurnBackend>,
    pub host_composition: Arc<HostComposition>,
    pub connector_registry: everruns_contracts::connector::ConnectorRegistry,
    pub provider_resolver: Arc<services::ProviderResolverService>,
    pub permission_resolver: Arc<dyn PermissionResolver>,
    pub sqldb_store: Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore>,
    pub org_rate_limiter: crate::auth::rate_limit::OrgRateLimiter,
    pub virtual_registry: Arc<VirtualMountRegistry>,
    pub slack_provisioner: Option<Arc<dyn SlackAppProvisioner>>,
}

/// Validate gRPC auth and TLS config, then supervise the worker gRPC server.
pub(super) fn spawn_grpc_server(
    supervisor: &mut TaskSupervisor,
    grpc_addr: &str,
    deps: WorkerLinkDeps,
    task_broadcaster: Option<Arc<crate::live_updates::task_notifications::TaskBroadcaster>>,
) -> Result<()> {
    let grpc_addr: std::net::SocketAddr = grpc_addr
        .parse()
        .context("Invalid SERVER_GRPC_BIND_ADDR/WORKER_GRPC_ADDR")?;
    let grpc_token = grpc_service::require_grpc_auth_token_result()
        .context("Invalid gRPC authentication configuration")?;
    let grpc_tls_config = grpc_service::grpc_server_tls_from_env_result()
        .context("Invalid gRPC TLS configuration")?;
    if let Some(tls) = grpc_tls_config.clone() {
        tonic::transport::Server::builder()
            .tls_config(tls)
            .context("Invalid gRPC TLS configuration")?;
    }

    supervisor.spawn(
        "grpc_server",
        RestartPolicy::always_after(Duration::from_secs(5)),
        move || {
            let deps = deps.clone();
            let task_broadcaster = task_broadcaster.clone();
            let grpc_token = grpc_token.clone();
            let grpc_tls_config = grpc_tls_config.clone();

            async move {
                let mut grpc_svc = grpc_service::WorkerServiceImpl::with_virtual_registry(
                    (*deps.event_service).clone(),
                    deps.db,
                    deps.encryption,
                    Some(deps.runner),
                    deps.host_composition.as_ref().clone(),
                    Some(deps.virtual_registry),
                    Some(deps.provider_resolver),
                );
                if let Some(broadcaster) = task_broadcaster {
                    grpc_svc.set_task_broadcaster(broadcaster);
                }
                grpc_svc.set_slack_provisioner(deps.slack_provisioner);
                grpc_svc.set_connector_registry(deps.connector_registry);
                grpc_svc.set_permission_resolver(deps.permission_resolver);
                // EVE-1047: the worker and the HTTP routes share one store.
                grpc_svc.set_sqldb_store(deps.sqldb_store);
                grpc_svc.set_org_rate_limiter(Arc::new(deps.org_rate_limiter));
                // THREAT[TM-DURABLE-002]: gRPC unauthenticated access
                // Mitigation: Bearer token auth + optional mTLS validated before spawn.
                let auth_interceptor = grpc_service::GrpcAuthInterceptor::new(Some(grpc_token));
                tracing::info!("gRPC server listening on {}", grpc_addr);

                let mut builder = tonic::transport::Server::builder();
                if let Some(tls) = grpc_tls_config {
                    match builder.tls_config(tls) {
                        Ok(tls_builder) => builder = tls_builder,
                        Err(e) => {
                            tracing::error!(error = %e, "Invalid gRPC TLS configuration");
                            return;
                        }
                    }
                }
                if let Err(e) = builder
                    .layer(tonic::service::interceptor::InterceptorLayer::new(
                        auth_interceptor,
                    ))
                    .add_service(grpc_svc.into_server())
                    .serve(grpc_addr)
                    .await
                {
                    tracing::error!(error = %e, "gRPC server error");
                }
            }
        },
    );
    Ok(())
}

/// Extra services only the in-process DEV_MODE worker wires directly.
pub(super) struct DevWorkerExtras {
    pub budget_service: Arc<crate::domains::budgets::BudgetService>,
    pub durable_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    pub connection_resolver:
        Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
}

/// DEV MODE: run the task worker in-process over `DirectWorkerAdapters`.
pub(super) fn spawn_dev_task_worker(
    supervisor: &mut TaskSupervisor,
    shared_store: Arc<PostgresWorkflowEventStore>,
    deps: WorkerLinkDeps,
    extras: DevWorkerExtras,
) {
    tracing::info!("DEV MODE: Starting task worker for in-process execution");

    let db = deps.db.clone();
    let encryption = deps.encryption.clone();
    let host_composition = deps.host_composition.clone();
    let mcp_server_service = Arc::new(
        crate::domains::mcp_servers::McpServerService::with_egress_service(
            db.clone(),
            encryption.clone(),
            host_composition.egress_service(),
        ),
    );
    let session_storage_store: Arc<dyn everruns_core::session_services::SessionStorageStore> = {
        let database = db.database();
        if let Some(enc) = &encryption {
            Arc::new(crate::storage::create_db_session_storage_store(
                database.clone(),
                enc.as_ref().clone(),
            ))
        } else {
            Arc::new(
                crate::storage::create_db_session_storage_store_without_encryption(
                    database.clone(),
                ),
            )
        }
    };

    let mut adapters = DirectWorkerAdapters::new(
        db,
        deps.event_service,
        deps.provider_resolver,
        mcp_server_service,
        (*host_composition.capability_registry()).clone(),
        host_composition.driver_registry().clone(),
        deps.sqldb_store,
    )
    .with_slack_provisioner(deps.slack_provisioner)
    .with_connector_registry(deps.connector_registry)
    .with_budget_service(extras.budget_service)
    .with_encryption(encryption)
    .with_workflow_store(extras.durable_store)
    .with_permission_resolver(deps.permission_resolver)
    .with_utility_llm_service(host_composition.utility_llm_service())
    .with_egress_service(host_composition.egress_service())
    .with_virtual_registry(deps.virtual_registry)
    .with_storage_store(session_storage_store)
    .with_runner(deps.runner)
    .with_vector_store(
        host_composition
            .extension::<everruns_capabilities::VectorStoreExt>()
            .expect("OSS platform definition installs a vector store")
            .0
            .clone(),
    )
    .with_org_rate_limiter(Arc::new(deps.org_rate_limiter));

    // Wire lazy connection resolver (requires encryption for token decryption).
    // Without encryption (e.g. DEV_MODE without SECRETS_ENCRYPTION_KEY) we cannot
    // decrypt stored tokens, so install a no-op resolver instead of leaving the
    // slot empty — `runtime_host::connection_resolver()` always calls into the
    // adapter at runtime and would otherwise panic.
    let connection_resolver = extras
        .connection_resolver
        .unwrap_or_else(|| Arc::new(crate::storage::NoopConnectionResolver));
    adapters = adapters
        .with_connection_resolver(connection_resolver)
        .with_dev_mode_in_memory_compaction_checkpoints();

    let worker_config = TaskWorkerConfig::dev_mode();
    supervisor.spawn(
        "dev_task_worker",
        RestartPolicy::always_after(Duration::from_secs(5)),
        move || {
            let worker_config = worker_config.clone();
            let shared_store = shared_store.clone();
            let adapters = adapters.clone();
            async move {
                let mut worker = TaskWorker::new(worker_config, shared_store, adapters);
                if let Err(e) = worker.run().await {
                    tracing::error!(error = %e, "Task worker error");
                }
            }
        },
    );

    tracing::info!("DEV MODE: Task worker started - server is fully functional");
}

/// Periodic provider model discovery. `MODEL_SYNC_INTERVAL_HOURS=0` disables it.
pub(super) fn spawn_model_sync(
    supervisor: &mut TaskSupervisor,
    db: Arc<StorageBackend>,
    driver_registry: Arc<everruns_contracts::driver_registry::DriverRegistry>,
    encryption: Option<Arc<EncryptionService>>,
) {
    let sync_interval_hours: u64 = std::env::var("MODEL_SYNC_INTERVAL_HOURS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);

    if sync_interval_hours == 0 {
        tracing::info!("Model sync background task disabled (MODEL_SYNC_INTERVAL_HOURS=0)");
        return;
    }

    let sync_service = Arc::new(domains::models::ModelSyncService::new(
        db,
        driver_registry,
        encryption,
    ));
    let sync_interval = Duration::from_secs(sync_interval_hours * 3600);

    supervisor.spawn(
        "model_sync",
        RestartPolicy::always_after(Duration::from_secs(5)),
        move || {
            let sync_service = sync_service.clone();
            async move {
                let mut interval = tokio::time::interval(sync_interval);
                interval.tick().await;

                tracing::info!(
                    interval_hours = sync_interval_hours,
                    "Started model discovery sync background task"
                );

                loop {
                    interval.tick().await;
                    tracing::info!("Starting scheduled model sync for all providers");
                    match sync_service.sync_all().await {
                        Ok(results) => {
                            for (provider_id, result) in results {
                                log_sync_result(provider_id, result);
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to run model sync");
                        }
                    }
                }
            }
        },
    );
}

fn log_sync_result(provider_id: impl std::fmt::Display, result: domains::models::SyncResult) {
    match result {
        domains::models::SyncResult::Success {
            created,
            updated,
            stale,
        } => {
            tracing::info!(
                %provider_id,
                created,
                updated,
                stale,
                "Model sync completed for provider"
            );
        }
        domains::models::SyncResult::NotSupported => {
            tracing::debug!(%provider_id, "Model sync not supported for provider");
        }
        domains::models::SyncResult::Failed { error } => {
            tracing::warn!(%provider_id, %error, "Model sync failed for provider");
        }
    }
}

/// Inputs for the cluster-wide maintenance loops below. They all run on the
/// background pool (EVE-1081) so they never queue behind request traffic.
pub(super) struct MaintenanceDeps {
    pub background_db: Arc<StorageBackend>,
    pub background_pool: sqlx::PgPool,
    pub background_runner: Arc<dyn everruns_core::host::TurnBackend>,
    pub background_event_service: Arc<services::EventService>,
    pub background_session_schedule_service:
        Arc<crate::domains::session_schedules::SessionScheduleService>,
    pub event_delivery: crate::live_updates::event_delivery::EventDelivery,
    pub connection_resolver:
        Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
    pub provider_resolver: Arc<services::ProviderResolverService>,
    pub driver_registry: Arc<everruns_contracts::driver_registry::DriverRegistry>,
    pub host_composition: Arc<HostComposition>,
}

/// Tool result timeouts, the session schedule poller, cluster-once jobs, and
/// reporting. Without a durable store (no scheduler) the tool result backstop
/// runs as a local sweep instead of a cluster job.
pub(super) async fn start_maintenance(
    supervisor: &mut TaskSupervisor,
    deps: MaintenanceDeps,
    cluster_jobs_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
) {
    // -- Tool result timeouts: a durable deadline task per parked turn, plus a backstop sweep --
    let tool_result_timeouts = crate::background::tool_result_timeout::ToolResultTimeouts::new(
        deps.background_db.clone(),
        deps.background_runner.clone(),
        deps.event_delivery.clone(),
    );

    // -- Session schedule poller (both prod and dev) --
    // Provide a built-in probe registry so monitors with a `spec["tool"]`
    // can run their probe directly without delegating to an agent turn.
    let probe_registry =
        std::sync::Arc::new(crate::background::session_scheduler::monitor_probe_tool_registry());
    supervisor.track(
        "session_scheduler",
        crate::background::session_scheduler::spawn_session_scheduler(
            deps.background_db.clone(),
            deps.background_session_schedule_service,
            deps.background_event_service,
            deps.background_runner,
            Some(probe_registry),
            crate::background::session_scheduler::poll_interval_from_env(),
        ),
    );

    // -- Cluster-once maintenance jobs on durable schedules (crate::background::cluster_jobs) --
    // Blob GC, event and Sandbox history retention, both source syncs: one run per cluster
    // per interval, whatever the replica count.
    let cluster_jobs = vec![
        crate::background::blob_gc::blob_gc_job(
            deps.background_db.clone(),
            crate::background::blob_gc::BlobGcConfig::from_env(),
        ),
        crate::background::event_retention::retention_job(
            Some(deps.background_pool.clone()),
            crate::background::event_retention::retention_days_from_env(),
        ),
        crate::background::sandbox_history_retention::retention_job(
            Some(deps.background_pool.clone()),
            crate::background::sandbox_history_retention::retention_days_from_env(),
        ),
        crate::background::session_trace_backfill::backfill_job(deps.background_db.clone()),
        crate::domains::memory::source_sync::memory_source_sync_job(
            deps.background_db.clone(),
            deps.connection_resolver.clone(),
        ),
        // Reuses Memory sync's GitHub connection resolver, the provider resolver, the
        // driver registry (embeddings), and the vector store:
        // knowledge/runtime-resources/knowledge-indexes.md
        crate::domains::knowledge_indexes::source_sync::knowledge_index_sync_job(
            deps.background_db.clone(),
            deps.connection_resolver,
            deps.provider_resolver.clone(),
            deps.driver_registry.clone(),
            deps.host_composition
                .extension::<everruns_capabilities::VectorStoreExt>()
                .expect("OSS platform definition installs a vector store")
                .0
                .clone(),
        ),
        tool_result_timeouts.backstop_job(),
    ];
    if let Some(store) = cluster_jobs_store {
        let tasks = vec![tool_result_timeouts.deadline_task(store.clone())];
        let pool = crate::background::cluster_jobs::start(store, cluster_jobs, tasks).await;
        supervisor.track_optional("cluster_jobs", pool);
    } else {
        let sweep = tool_result_timeouts.spawn_local_sweep();
        supervisor.track("tool_result_sweep", sweep);
    }

    // -- Reporting projection and missing-work reconciliation (both prod and dev) --
    for handle in crate::domains::reporting::background::spawn_reporting_background_task(
        deps.background_db.clone(),
    ) {
        supervisor.track("reporting_background", handle);
    }
}
