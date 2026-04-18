// Audit log query helpers — shared by commands and HTTP adapters.
//
// No policy checks, no input validation. Pure data access + mapping.
//
// The storage → domain view mapper (`AuditLogRow → AuditLogEntry`) lives in
// `types.rs` via `impl From<AuditLogRow> for AuditLogEntry`. This module is
// kept empty for parity with other domains and as the landing site for future
// shared read helpers (e.g. targeted fetches or cross-command helpers).
