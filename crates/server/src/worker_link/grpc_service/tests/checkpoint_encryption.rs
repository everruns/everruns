//! A remote gRPC worker runs turns without the checkpoint stores when the
//! control plane has no `SECRETS_ENCRYPTION_KEY`, like the in-process worker,
//! instead of failing every turn on the refused checkpoint read (EVE-1261).

use super::*;
use everruns_core::CompactionCheckpointStore;
use everruns_worker::GrpcWorkerAdapters;
use everruns_worker::grpc_adapters::GrpcClient;
use everruns_worker::worker_adapters::WorkerAdapters;

fn worker_service(encryption: Option<Arc<EncryptionService>>) -> WorkerServiceImpl {
    let db = Arc::new(StorageBackend::test_database());
    let host_composition =
        crate::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev);
    let event_service =
        EventService::with_listeners(db.clone(), crate::EventDelivery::in_memory(), vec![]);
    WorkerServiceImpl::new(event_service, db, encryption, None, host_composition)
}

fn test_encryption() -> Arc<EncryptionService> {
    Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
            .expect("valid test encryption key"),
    )
}

fn store_presence(adapters: &GrpcWorkerAdapters) -> [bool; 3] {
    [
        adapters.compaction_checkpoint_store().is_some(),
        adapters.native_async_store().is_some(),
        adapters.agents_api_store().is_some(),
    ]
}

async fn latest_checkpoint(
    store: &dyn CompactionCheckpointStore,
) -> everruns_contracts::error::Result<Option<everruns_core::CompactionCheckpoint>> {
    store
        .get_latest(
            everruns_contracts::typed_id::SessionId::new(),
            "openai",
            "gpt-6.1-sol",
        )
        .await
}

#[tokio::test]
async fn worker_without_server_encryption_runs_without_checkpoint_stores() {
    let (addr, shutdown, server) = start_grpc_test_server(worker_service(None)).await;

    let adapters = GrpcWorkerAdapters::connect(&addr).await.unwrap();
    assert_eq!(store_presence(&adapters), [false; 3]);

    // A worker that skipped the connect probe learns it from the first
    // refused read: that read finds no checkpoint instead of failing the turn.
    let late = GrpcWorkerAdapters::from_client(GrpcClient::connect(&addr).await.unwrap());
    let store = late
        .compaction_checkpoint_store()
        .expect("an unprobed worker still offers the store");
    let checkpoint = latest_checkpoint(store.as_ref())
        .await
        .expect("missing server encryption must not fail the checkpoint read");
    assert!(checkpoint.is_none());
    assert_eq!(store_presence(&late), [false; 3]);

    let _ = shutdown.send(());
    server.await.unwrap();
}

#[tokio::test]
async fn worker_with_server_encryption_keeps_checkpoint_stores() {
    let service = worker_service(Some(test_encryption()));
    let (addr, shutdown, server) = start_grpc_test_server(service).await;

    let adapters = GrpcWorkerAdapters::connect(&addr).await.unwrap();
    assert_eq!(store_presence(&adapters), [true; 3]);
    let store = adapters.compaction_checkpoint_store().unwrap();
    assert!(latest_checkpoint(store.as_ref()).await.unwrap().is_none());
    assert_eq!(store_presence(&adapters), [true; 3]);

    let _ = shutdown.send(());
    server.await.unwrap();
}
