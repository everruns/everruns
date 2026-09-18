//! Session SQL databases.
//!
//! Handler bodies for the `WorkerService` RPCs in this group. The trait impl in
//! `super::super::worker_service_impl` is a delegation layer only: a trait impl
//! cannot span modules, so the work lives here and the trait forwards to it.

use crate::grpc_service::*;

impl WorkerServiceImpl {
    pub(crate) async fn handle_session_sql_db_create_database(
        &self,
        request: Request<SessionSqlDbCreateDatabaseRequest>,
    ) -> Result<Response<SessionSqlDbCreateDatabaseResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let db = store
            .create_database(session_id.into(), &req.name)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbCreateDatabaseResponse {
            database: Some(db_info_to_proto(db)),
        }))
    }

    pub(crate) async fn handle_session_sql_db_list_databases(
        &self,
        request: Request<SessionSqlDbListDatabasesRequest>,
    ) -> Result<Response<SessionSqlDbListDatabasesResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let databases = store
            .list_databases(session_id.into())
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbListDatabasesResponse {
            databases: databases.into_iter().map(db_info_to_proto).collect(),
        }))
    }

    pub(crate) async fn handle_session_sql_db_get_database(
        &self,
        request: Request<SessionSqlDbGetDatabaseRequest>,
    ) -> Result<Response<SessionSqlDbGetDatabaseResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let db = store
            .get_database(session_id.into(), &req.name)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbGetDatabaseResponse {
            database: db.map(db_info_to_proto),
        }))
    }

    pub(crate) async fn handle_session_sql_db_delete_database(
        &self,
        request: Request<SessionSqlDbDeleteDatabaseRequest>,
    ) -> Result<Response<SessionSqlDbDeleteDatabaseResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let deleted = store
            .delete_database(session_id.into(), &req.name)
            .await
            .map_err(sqldb_error_to_status)?;

        Ok(Response::new(SessionSqlDbDeleteDatabaseResponse {
            deleted,
        }))
    }

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

    pub(crate) async fn handle_session_sql_db_schema(
        &self,
        request: Request<SessionSqlDbSchemaRequest>,
    ) -> Result<Response<SessionSqlDbSchemaResponse>, Status> {
        let req = request.into_inner();
        let session_id = parse_uuid(req.session_id.as_ref())?;
        let store = self.sqldb_store()?;

        let tables = store
            .sql_schema(session_id.into(), &req.db_name, req.table.as_deref())
            .await
            .map_err(sqldb_error_to_status)?;

        let proto_tables: Vec<proto::SessionSqlDbTableSchema> = tables
            .into_iter()
            .map(|t| proto::SessionSqlDbTableSchema {
                name: t.name,
                columns: t
                    .columns
                    .into_iter()
                    .map(|c| proto::SessionSqlDbColumnSchema {
                        name: c.name,
                        column_type: c.column_type,
                        notnull: c.notnull,
                        pk: c.pk,
                        default_value: c.default_value,
                    })
                    .collect(),
                row_count: t.row_count,
            })
            .collect();

        Ok(Response::new(SessionSqlDbSchemaResponse {
            tables: proto_tables,
        }))
    }
}
