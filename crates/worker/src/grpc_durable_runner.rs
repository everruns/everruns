// gRPC durable runner: the worker's control-plane transport for `DurableRunner`.
//
// Decision: durable-engine owns `DurableRunner` and the `TurnStore` trait but
// carries no transport. The worker owns `GrpcDurableStore`, implements that
// trait for it (`grpc_task_store`), and provides the gRPC constructors here.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::core::config::env_string_any;
use crate::durable_runner::DurableRunner;
use crate::grpc_durable_store::GrpcDurableStore;
use crate::runner::AgentRunner;

/// Connect a durable runner to the control plane's gRPC durable service.
pub async fn connect_grpc_durable_runner(grpc_address: &str) -> Result<DurableRunner> {
    info!(
        grpc_address = %grpc_address,
        "Initializing durable runner (gRPC mode)"
    );
    let store = GrpcDurableStore::connect(grpc_address).await?;
    Ok(DurableRunner::from_store(store))
}

/// Connect a durable runner to the control plane named by
/// `SERVER_GRPC_ADDRESS` / `WORKER_GRPC_ADDRESS` (default `127.0.0.1:9001`).
pub async fn grpc_durable_runner_from_env() -> Result<DurableRunner> {
    let grpc_address = env_string_any(
        &["SERVER_GRPC_ADDRESS", "WORKER_GRPC_ADDRESS"],
        "127.0.0.1:9001",
    );
    connect_grpc_durable_runner(&grpc_address).await
}

/// Create an agent runner.
///
/// Pass a database pool for direct access (control-plane) or `None` for the
/// gRPC control-plane transport (workers).
pub async fn create_runner(
    db_pool: Option<crate::durable::PostgresPool>,
) -> Result<Arc<dyn AgentRunner>> {
    if let Some(pool) = db_pool {
        tracing::info!("Creating Durable execution engine runner (direct DB mode)");
        Ok(Arc::new(DurableRunner::new_with_pool(pool)))
    } else {
        tracing::info!("Creating Durable execution engine runner (gRPC mode)");
        let runner = grpc_durable_runner_from_env().await?;
        Ok(Arc::new(runner))
    }
}
