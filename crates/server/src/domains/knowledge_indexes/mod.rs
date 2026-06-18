// Knowledge Indexes domain. See specs/knowledge-indexes.md.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod embedding;
pub mod search;
pub mod source_sync;
pub mod types;

pub use search::KnowledgeIndexSearchService;

pub use commands::*;

pub const KNOWLEDGE_INDEX_VIEW: Policy = Policy {
    id: "knowledge_index.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const KNOWLEDGE_INDEX_MANAGE: Policy = Policy {
    id: "knowledge_index.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

/// Allowed source types. Mirrors the CHECK constraint in
/// `crates/server/migrations/071_knowledge_indexes.sql`.
pub const SOURCE_TYPES: &[&str] = &["github", "git"];

/// Default source type when the request omits it.
pub const DEFAULT_SOURCE_TYPE: &str = "github";
