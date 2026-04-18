// Audit log query helpers — shared by commands and HTTP adapters.
//
// No policy checks, no input validation. Pure data access + mapping.

// Audit logs expose their storage row shape directly to the HTTP adapter;
// no row-to-domain mapper is required today. This file exists for parity
// with other domain modules and as the landing site for future mappers
// (e.g. an AuditLogEntry view model for the MCP catalog).
