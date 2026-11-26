// Memory v2 views over stored memories (knowledge/runtime-resources/memory.md).
//
// A memory row is one store. Its typed entries are Markdown files in kind
// folders; these commands read them for the console and the CLI, and list the
// versions every change leaves in `memory_file_versions`.
//
// Decisions:
// - The store a row is follows from its scope: `agent`, `user`, `team`, the
//   org's default memory (scope `org`, name `org`), or a named memory (any
//   other `org` row whose name is a store name). Rows that are none of these
//   (a run's team folder, a room, a legacy name) are not memory stores here.
// - A person's private memory is visible only to that person, whatever their
//   org role (THREAT[TM-TENANT-013]).
// - Entries that do not parse are listed with their error rather than hidden,
//   so someone can find and fix a file written by hand before the server
//   refused bad entry files.

use super::MEMORY_VIEW;
use super::types::{MemoryResponse, MemoryRow, memory_response};
use crate::domains::common::*;
use chrono::{DateTime, Utc};
use everruns_contracts::runtime::memory::{
    MemoryEntry, MemoryStoreKey, MemoryStoreScope, entry_location, parse_entry_file,
    validate_store_name,
};
use everruns_contracts::typed_id::MemoryId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// `memories.name` of the organization's default store.
pub const ORG_MEMORY_NAME: &str = crate::domains::session_files::memory_mounts::ORG_MEMORY_NAME;

/// The store a memory row is, when it is one.
pub fn store_key(row: &MemoryRow) -> Option<MemoryStoreKey> {
    match row.scope.as_str() {
        "agent" => Some(MemoryStoreKey::scope(MemoryStoreScope::Agent)),
        "user" => Some(MemoryStoreKey::scope(MemoryStoreScope::User)),
        "team" => Some(MemoryStoreKey::scope(MemoryStoreScope::Team)),
        "org" if row.name == ORG_MEMORY_NAME => Some(MemoryStoreKey::scope(MemoryStoreScope::Org)),
        "org" if validate_store_name(&row.name).is_ok() => {
            Some(MemoryStoreKey::named(row.name.clone()))
        }
        _ => None,
    }
}

/// Whether `viewer` may see `row`: everything but another person's private
/// memory.
pub fn visible_to(row: &MemoryRow, viewer: Option<Uuid>) -> bool {
    row.scope != "user" || (row.owner_user_id.is_some() && row.owner_user_id == viewer)
}

/// Refuse an entry file that would not parse (`MemoryFileService` writes).
pub fn check_entry_content(row: &MemoryRow, path: &str, content: &[u8]) -> Result<(), String> {
    let Some(store) = store_key(row) else {
        return Ok(());
    };
    if entry_location(path).is_none() {
        return Ok(());
    }
    let text = std::str::from_utf8(content)
        .map_err(|_| format!("{path}: a memory entry is UTF-8 Markdown"))?;
    parse_entry_file(&store, path, text)
        .map(|_| ())
        .map_err(|error| format!("not saved: {error}"))
}

/// One store, as the console lists it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryStoreView {
    #[serde(flatten)]
    pub memory: MemoryResponse,
    /// How agents name it: `agent`, `user`, `org`, `team`, or the memory's
    /// name.
    #[schema(example = "team")]
    pub store: String,
    /// Where it hangs in a session of an agent that uses it.
    #[schema(example = "/memory/team")]
    pub mount_path: String,
    /// Who it belongs to, in a sentence.
    pub about: String,
    /// The owning agent's name, for an agent or team memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// Typed entries (files in kind folders, proposals included).
    pub entry_count: usize,
    /// Every file, entries included.
    pub file_count: usize,
    /// Open blocked items.
    pub blocked_count: usize,
    /// Dreaming proposals waiting to be accepted.
    pub proposal_count: usize,
    /// When any file last changed; the store's creation when it has none.
    pub last_changed_at: DateTime<Utc>,
}

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListMemoryStores {}

#[command(
    name = "list_memory_stores",
    category = "memories",
    description = "List the memory stores agents use: the organization's, named memories, team and agent memories, and your own private memory.",
    method = "GET",
    path = "/v1/platform/memory/stores",
    policy = MEMORY_VIEW,
    cli = CliRoute::new(&["memories"], "stores").with_examples(&[CliExample::new("See which memories exist and how much each holds", "everruns memories stores",)]),
)]
impl Command for ListMemoryStores {
    type Output = Vec<MemoryStoreView>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<MemoryStoreView>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<MemoryStoreView>, CommandError> {
        let viewer = ctx.caller.user_id;
        let rows = ctx.db.list_memory_stores(ctx.org_id()).await?;
        let owners: std::collections::HashMap<Uuid, String> = ctx
            .db
            .memory_owner_agent_names(ctx.org_id())
            .await?
            .into_iter()
            .collect();
        let mut out = Vec::new();
        for row in rows {
            if !visible_to(&row, viewer) {
                continue;
            }
            let Some(store) = store_key(&row) else {
                continue;
            };
            let files = ctx.db.list_all_memory_files(row.id).await?;
            let mut entry_count = 0;
            let mut blocked_count = 0;
            let mut proposal_count = 0;
            let mut file_count = 0;
            let mut last_changed_at = row.created_at;
            for file in files.iter().filter(|file| !file.is_directory) {
                file_count += 1;
                last_changed_at = last_changed_at.max(file.updated_at);
                let Some((kind, proposal)) = entry_location(&file.path) else {
                    continue;
                };
                entry_count += 1;
                if proposal {
                    proposal_count += 1;
                } else if kind == everruns_contracts::runtime::memory::MemoryEntryKind::Blocked
                    && let Some(content) = &file.content
                    && let Ok(text) = std::str::from_utf8(content)
                    && let Ok(entry) = parse_entry_file(&store, &file.path, text)
                    && entry.is_open()
                {
                    blocked_count += 1;
                }
            }
            let about = match store.scope {
                MemoryStoreScope::Agent => "One agent's own memory, shared by all its sessions.",
                MemoryStoreScope::User => "Your private memory; only your own sessions read it.",
                MemoryStoreScope::Org => {
                    "The organization's memory; every agent with Memory reads it."
                }
                MemoryStoreScope::Team => {
                    "A team's memory: its coordinator and every thread or agent it starts."
                }
                MemoryStoreScope::Named => {
                    "A named memory, shared by the agents it is attached to."
                }
                MemoryStoreScope::Parent => "",
            }
            .to_string();
            out.push(MemoryStoreView {
                owner_name: owners.get(&row.id).cloned(),
                mount_path: store.mount_path(),
                store: store.label(),
                about,
                entry_count,
                file_count,
                blocked_count,
                proposal_count,
                last_changed_at,
                memory: memory_response(row).map_err(classify_anyhow)?,
            });
        }
        Ok(out)
    }
}

/// An entry file that does not parse.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvalidMemoryEntry {
    pub path: String,
    pub error: String,
}

/// A store's entries.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryEntriesView {
    pub store: String,
    /// Newest first.
    pub entries: Vec<MemoryEntry>,
    pub invalid: Vec<InvalidMemoryEntry>,
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListMemoryEntries {
    /// Memory's prefixed public identifier.
    pub memory_id: String,
}

async fn visible_memory(ctx: &Ctx, memory_id: &str) -> Result<MemoryRow, CommandError> {
    let id: MemoryId = memory_id
        .parse()
        .map_err(|_| CommandError::not_found("Memory"))?;
    ctx.db
        .get_memory(ctx.org_id(), id)
        .await?
        .filter(|row| visible_to(row, ctx.caller.user_id) && row.scope != "room")
        .ok_or_else(|| CommandError::not_found("Memory"))
}

#[command(
    name = "list_memory_entries",
    category = "memories",
    description = "List a memory's typed entries: decisions, rules, facts, preferences, blocked items, notes and dreaming proposals.",
    method = "GET",
    path = "/v1/platform/memories/{memory_id}/entries",
    policy = MEMORY_VIEW,
    positional = "memory_id",
    cli = CliRoute::new(&["memories"], "entries").with_args(&[CliArg::new("memory_id").at(1)]).with_examples(&[CliExample::new("Read what a team decided and what it is blocked on", "everruns memories entries mem_01h9",)]),
)]
impl Command for ListMemoryEntries {
    type Output = MemoryEntriesView;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<MemoryEntriesView>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<MemoryEntriesView, CommandError> {
        let row = visible_memory(ctx, &self.memory_id).await?;
        let store = store_key(&row)
            .ok_or_else(|| CommandError::bad_request("this memory holds no typed entries"))?;
        let mut entries = Vec::new();
        let mut invalid = Vec::new();
        for file in ctx.db.list_all_memory_files(row.id).await? {
            if file.is_directory || entry_location(&file.path).is_none() {
                continue;
            }
            let content = file.content.unwrap_or_default();
            let parsed = std::str::from_utf8(&content)
                .map_err(|_| "not UTF-8 text".to_string())
                .and_then(|text| {
                    parse_entry_file(&store, &file.path, text).map_err(|e| e.to_string())
                });
            match parsed {
                Ok(entry) => entries.push(entry),
                Err(error) => invalid.push(InvalidMemoryEntry {
                    path: file.path,
                    error,
                }),
            }
        }
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.occurred_at));
        Ok(MemoryEntriesView {
            store: store.label(),
            entries,
            invalid,
        })
    }
}

/// One version of a memory file.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryFileVersion {
    pub path: String,
    /// `write` or `delete`.
    pub action: String,
    /// The text after the change; absent for a delete or a binary file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub size_bytes: i64,
    /// Who changed it, when known (`session:<id>`, `user:<id>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize, utoipa::IntoParams)]
pub struct MemoryFileHistory {
    /// Memory's prefixed public identifier.
    pub memory_id: String,
    /// A path inside the memory (`/decisions/everruns-5528.md`). Omit for
    /// the memory's latest changes across every file.
    #[serde(default)]
    pub path: Option<String>,
}

#[command(
    name = "memory_file_history",
    category = "memories",
    description = "List the versions of a memory file, newest first; every write and delete is kept.",
    method = "GET",
    path = "/v1/platform/memories/{memory_id}/history",
    policy = MEMORY_VIEW,
    positional = "memory_id",
    cli = CliRoute::new(&["memories"], "history").with_args(&[CliArg::new("memory_id").at(1)]).with_examples(&[CliExample::new("See every version of one decision", "everruns memories history mem_01h9 --path /decisions/everruns-5528.md",)]),
)]
impl Command for MemoryFileHistory {
    type Output = Vec<MemoryFileVersion>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<MemoryFileVersion>())
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<MemoryFileVersion>, CommandError> {
        let row = visible_memory(ctx, &self.memory_id).await?;
        let path = self
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| {
                if path.starts_with('/') {
                    path.to_string()
                } else {
                    format!("/{path}")
                }
            });
        Ok(ctx
            .db
            .list_memory_file_versions(row.id, path.as_deref(), 200)
            .await?
            .into_iter()
            .map(|version| MemoryFileVersion {
                content: version
                    .content
                    .and_then(|bytes| String::from_utf8(bytes).ok()),
                path: version.path,
                action: version.action,
                size_bytes: version.size_bytes,
                changed_by: version.changed_by,
                created_at: version.created_at,
            })
            .collect())
    }
}
