// Knowledge Bases domain. See specs/knowledge-bases.md.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod okf;
pub mod types;

pub use commands::*;

pub const KNOWLEDGE_BASE_VIEW: Policy = Policy {
    id: "knowledge_base.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const KNOWLEDGE_BASE_MANAGE: Policy = Policy {
    id: "knowledge_base.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

/// Allowed entry kinds. Mirrors the CHECK constraint in
/// `crates/server/migrations/032_knowledge_bases.sql`.
pub const ENTRY_KINDS: &[&str] = &["note", "table", "business", "query", "runbook"];

/// Hard cap on entry body size, matching the SQL `octet_length(body) <= 65536`
/// constraint. Surfaced at the domain layer so the API returns a 400 instead
/// of a database error.
pub const MAX_ENTRY_BODY_BYTES: usize = 65_536;

/// Tag count limit, matching the SQL `cardinality(tags) <= 16` constraint.
pub const MAX_ENTRY_TAGS: usize = 16;
