//! Session SQL databases.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_session_sql_db_execute(
        &self,
        request: Request<SessionSqlDbExecuteRequest>,
    ) -> Result<Response<SessionSqlDbExecuteResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let result = store
            .sql_execute(session_id.into(), &req.db_name, &req.sql)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbExecuteResponse {
            rows_affected: result.rows_affected,
        }))
    }

    pub(crate) async fn handle_session_sql_db_query(
        &self,
        request: Request<SessionSqlDbQueryRequest>,
    ) -> Result<Response<SessionSqlDbQueryResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let result = store
            .sql_query(session_id.into(), &req.db_name, &req.sql)
            .await
            .map_err(sqldb_error_to_status)?;

        let proto_rows: Vec<prost_types::ListValue> = result
            .rows
            .into_iter()
            .map(|row| prost_types::ListValue {
                values: row.into_iter().map(json_value_to_proto).collect(),
            })
            .collect();

        Ok(Response::new(SessionSqlDbQueryResponse {
            columns: result.columns,
            rows: proto_rows,
            row_count: result.row_count as u64,
            truncated: result.truncated,
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::grpc_service::tests::{
        create_grpc_test_session, start_grpc_test_server, test_worker_service,
    };
    use everruns_platform::session_sqldb::SessionSqlDbStore;
    use std::sync::Arc;

    /// The session-database CRUD operations reach the server through
    /// `ExecuteCommand` rather than bespoke RPCs. The worker calls with
    /// `user_id: None`, so the server resolves `Caller::internal(org_id)`.
    ///
    /// End to end on purpose: the unit tests either side of this boundary both
    /// passed while the command `Ctx` was built without a sqldb store, which
    /// failed every one of these calls with "not configured". Only a real client
    /// against a real service catches that.
    #[tokio::test]
    async fn session_database_crud_runs_over_the_command_transport() {
        let service = test_worker_service().await;
        let (session_id, _harness_id) = create_grpc_test_session(&service).await;

        let (addr, shutdown_tx, server) = start_grpc_test_server(service).await;
        let client = everruns_worker::GrpcClient::connect(&addr)
            .await
            .expect("worker grpc client should connect");
        let store: Arc<dyn SessionSqlDbStore> =
            Arc::new(everruns_worker::grpc_adapters::GrpcAdapter::new_org_scoped(
                client,
                everruns_core::DEFAULT_ORG_ID,
            ));

        let created = store
            .create_database(session_id, "notes")
            .await
            .expect("create_session_database over the command transport");
        assert_eq!(created.name, "notes");

        let listed = store.list_databases(session_id).await.expect("list");
        assert!(
            listed.iter().any(|db| db.name == "notes"),
            "created database missing from the listing: {listed:?}"
        );

        let fetched = store.get_database(session_id, "notes").await.expect("get");
        assert_eq!(fetched.map(|db| db.name), Some("notes".to_string()));

        // A missing database is NotFound over the command transport and `None`
        // in the store contract.
        let absent = store
            .get_database(session_id, "absent")
            .await
            .expect("get absent");
        assert!(
            absent.is_none(),
            "expected None for a database that does not exist"
        );

        let schema = store
            .sql_schema(session_id, "notes", None)
            .await
            .expect("schema");
        assert!(
            schema.is_empty(),
            "a fresh database has no tables: {schema:?}"
        );

        assert!(
            store
                .delete_database(session_id, "notes")
                .await
                .expect("delete")
        );
        assert!(
            store
                .get_database(session_id, "notes")
                .await
                .expect("get after delete")
                .is_none()
        );

        let _ = shutdown_tx.send(());
        let _ = server.await;
    }
}
