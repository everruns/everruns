// Rows the compaction checkpoints repository reads and writes.

#[derive(Debug, Clone, sqlx::FromRow, everruns_server_macros::Columns)]
pub struct CompactionCheckpointRow {
    pub id: uuid::Uuid,
    pub session_id: everruns_contracts::typed_id::SessionId,
    pub source_sequence: i32,
    pub provider_type: String,
    pub model: String,
    pub format_version: i32,
    pub payload_encrypted: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct InstallCompactionCheckpointRow {
    pub id: uuid::Uuid,
    pub session_id: everruns_contracts::typed_id::SessionId,
    pub source_sequence: i32,
    pub provider_type: String,
    pub model: String,
    pub format_version: i32,
    pub payload_encrypted: Vec<u8>,
}
