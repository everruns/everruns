//! EVE-1047: the worker's session SQL database store is the HTTP app's store.
//!
//! Its own file rather than another block in `tests.rs`, which is on the size
//! ratchet's debt list and may not grow.

use super::tests::test_worker_service;
use super::*;

/// EVE-1047: the worker's store and the HTTP app's store are one store.
///
/// The constructor builds its own in-memory backend, which is right when the
/// gRPC service stands alone (tests, the direct adapters) and wrong in the
/// composed server, where `app_builder` spawns it beside the HTTP routes. When
/// they diverged, a database an agent created was invisible to
/// `GET /v1/sessions/{id}/databases` — which answered `200` with an empty list,
/// so nothing looked broken from either side.
#[tokio::test]
async fn injected_sqldb_store_replaces_the_services_own() {
    use everruns_platform::session_sqldb::SessionSqlDbStore;

    let mut service = test_worker_service().await;
    let session_id = everruns_provider::typed_id::SessionId::from_uuid(uuid::Uuid::now_v7());

    // Stand in for the HTTP app's store: create a database only it knows about.
    let app_store: Arc<dyn SessionSqlDbStore> =
        Arc::new(crate::session_sqldb::InMemorySqlDbStore::new(Arc::new(
            crate::session_sqldb::InMemorySqlDbBackend::new(),
        )));
    app_store
        .create_database(session_id, "smoke_db")
        .await
        .expect("create through the app's store");

    // Before injection the service cannot see it — this is the bug.
    let own = service
        .sqldb_store()
        .expect("the constructor always installs one")
        .list_databases(session_id)
        .await
        .expect("list through the service's own store");
    assert!(
        own.is_empty(),
        "the constructor's backend starts empty; sharing is what this test is about"
    );

    service.set_sqldb_store(app_store.clone());

    let shared = service
        .sqldb_store()
        .expect("store present after injection")
        .list_databases(session_id)
        .await
        .expect("list through the injected store");
    assert!(
        shared.iter().any(|db| db.name == "smoke_db"),
        "the worker must see what the HTTP app created: {shared:?}"
    );
}
