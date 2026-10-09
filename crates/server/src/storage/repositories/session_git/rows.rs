// Rows the session git repository reads and writes.

use crate::kernel_imports::contracts::typed_id::SessionId;
use sqlx::FromRow;

/// Git object row from database (session-scoped)
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SessionGitObjectRow {
    pub session_id: SessionId,
    pub oid: Vec<u8>,  // 20-byte SHA1
    pub obj_type: i16, // 1=commit, 2=tree, 3=blob, 4=tag
    pub size: i64,
    pub data: Vec<u8>,
}

/// Input for writing a git object
#[derive(Debug, Clone)]
pub struct CreateSessionGitObject {
    pub session_id: SessionId,
    pub oid: Vec<u8>,
    pub obj_type: i16,
    pub size: i64,
    pub data: Vec<u8>,
}

/// Git ref stored in PostgreSQL (session-scoped refdb)
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SessionGitRefRow {
    pub session_id: SessionId,
    pub name: String,
    pub target: Vec<u8>, // 20-byte oid or symbolic target
    pub is_symbolic: bool,
}

/// Input for writing a git ref
#[derive(Debug, Clone)]
pub struct CreateSessionGitRef {
    pub session_id: SessionId,
    pub name: String,
    pub target: Vec<u8>,
    pub is_symbolic: bool,
}
