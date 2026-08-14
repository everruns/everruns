//! Product composition for the public Scale control plane.

use std::sync::Arc;

use anyhow::Result;
use everruns_core::config::env_string_any;
use everruns_durable::InMemoryWorkflowEventStore;
use everruns_scale::{DurableTaskNotifier, DurableTurnDriver, RunController, ScaleEngine, migrate};

use crate::durable_runner::GrpcDurableStoreAdapter;

pub enum RunnerBackend {
    Postgres(sqlx::PgPool),
    PostgresWithNotifier {
        pool: sqlx::PgPool,
        task_notifier: Arc<dyn DurableTaskNotifier>,
    },
    InMemory,
    SharedInMemory(Arc<InMemoryWorkflowEventStore>),
    Grpc,
}

pub async fn create_runner(db_pool: Option<sqlx::PgPool>) -> Result<Arc<dyn RunController>> {
    match db_pool {
        Some(pool) => create_runner_with_backend(RunnerBackend::Postgres(pool)).await,
        None => create_runner_with_backend(RunnerBackend::Grpc).await,
    }
}

pub async fn create_runner_with_backend(backend: RunnerBackend) -> Result<Arc<dyn RunController>> {
    let engine = match backend {
        RunnerBackend::Postgres(pool) => {
            migrate(&pool).await?;
            ScaleEngine::from_controller(DurableTurnDriver::postgres(pool))
        }
        RunnerBackend::PostgresWithNotifier {
            pool,
            task_notifier,
        } => {
            migrate(&pool).await?;
            ScaleEngine::from_controller(
                DurableTurnDriver::postgres(pool).with_notifier(task_notifier),
            )
        }
        RunnerBackend::InMemory => ScaleEngine::from_controller(DurableTurnDriver::in_memory()),
        RunnerBackend::SharedInMemory(store) => ScaleEngine::in_memory_controller(store),
        RunnerBackend::Grpc => {
            let address = env_string_any(
                &["SERVER_GRPC_ADDRESS", "WORKER_GRPC_ADDRESS"],
                "127.0.0.1:9001",
            );
            let store = GrpcDurableStoreAdapter::connect(&address).await?;
            ScaleEngine::from_controller(DurableTurnDriver::from_store(store))
        }
    };
    Ok(Arc::new(engine))
}
