// Session SQL database types
//
// Re-exports from core. The canonical type definitions live in
// everruns_core::session_sqldb. This module keeps backward compatibility.

pub use everruns_core::session_sqldb::{
    ColumnSchema, DatabaseInfo, SqlExecuteResult, SqlQueryResult, TableSchema,
};
