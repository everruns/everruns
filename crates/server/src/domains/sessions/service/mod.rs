// Session service for business logic (M2)
//
// Design Decision: Capability mounts are applied at session creation time.
// This ensures mounted files are available immediately when the session starts.
// The service collects mounts from the agent's capabilities and applies them
// to the session filesystem. Session capabilities are applied after agent capabilities
// (additive behavior).

use super::types::{SessionFacetCount, SessionFacetsResponse};
use crate::api::common::Pagination;
use crate::domains::harnesses::queries::resolve_effective as resolve_effective_harness;
use crate::domains::session_files::{CreateFileInput, WorkspaceFileService};
use crate::domains::session_sandbox::SessionSandboxService;
use crate::domains::sessions::limits::OrgCaps;
use crate::errors::{BadRequestError, ResourceLimitError, ResourceNotFoundError};
use crate::kernel_imports::{
    AgentCapabilityConfig, Caller, CapabilityRegistry, everruns_provider::typed_id::AgentId,
};
use crate::kernel_imports::{
    DeclarativeCapabilityDefinition, InitialFile, MountAccess, MountEntry, MountPoint, MountSource,
    OrgRole, Permission, Policy, PrincipalSummary, Rule, SessionFile, SessionSeedMode, TokenUsage,
    capabilities::{
        RiskLevel, SystemPromptContext, collect_capabilities_with_configs, compute_features,
        resolve_capability_configs,
    },
    everruns_provider::typed_id::HarnessId,
    everruns_provider::typed_id::ModelId,
    everruns_provider::typed_id::PrincipalId,
    everruns_provider::typed_id::SessionId,
    everruns_provider::typed_id::WorkspaceId,
    is_declarative_capability, is_plugin_capability, is_skill_capability, merge_capabilities,
    merge_initial_files, normalize_initial_file_path, parse_declarative_capability_id,
    parse_skill_capability_id,
};
use crate::max_iterations;
use crate::org_init;
use crate::server::ResourceLimitsConfig;
use crate::services::{PrincipalService, row_to_principal};
use crate::storage::{
    StorageBackend,
    models::{
        CreateEventRow, CreateMemoryRow, CreateSessionFileRow, CreateSessionRow, MemoryFileRow,
        MemoryRow, SessionListFilters, UpdateSession, UpsertSessionKeyValue, UpsertSessionSecret,
    },
};
use anyhow::Result;
use everruns_builtins::AttachSkillCapability;
use everruns_durable::UpdateField;
use everruns_mcp::is_mcp_capability;
use everruns_platform::FeatureFlags;
use everruns_platform::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID;
use everruns_platform::{
    AgentVersionPolicy, MemoryConfig, MemoryMountAccess, capabilities::MEMORY_CAPABILITY_ID,
};
use everruns_platform::{Session, SessionActivity, SessionSource, SessionStatus};
use everruns_provider::typed_id::MemoryId;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::sessions::{CreateSessionRequest, UpdateSessionRequest};

const AGENT_MEMORY_MOUNT_PATH: &str = "/memory/agent";
const USER_MEMORY_MOUNT_PATH: &str = "/memory/user";
// THREAT[TM-AUTHZ-009][TM-A2A-007]: Session reuse and budget attribution match these routing
// namespaces. This list is append-only: removing a retired prefix would let external callers forge
// tags that older routing paths can still match.
const RESERVED_SESSION_TAG_PREFIXES: &[&str] = &[
    "__internal:",
    "app:",
    "app_channel:",
    "slack:app:",
    // Per-transport endpoint namespaces. Added with EVE-1004, which made them
    // attribution inputs: migration 137 reads all three endpoint tag spellings
    // to decide which endpoint a session arrived through. Not exploitable
    // before this — every routing lookup also anchors on `app_id`, which only
    // internal callers can set — but the list is what keeps that true as
    // readers are added.
    "slack:endpoint:",
    "fcp:endpoint:",
    "ag_ui:app:",
    "agent:",
    "endpoint:",
];
const RESERVED_SESSION_TAG_ERROR: &str = "Tags with '__internal:', 'app:', 'app_channel:', \
    'slack:app:', 'slack:endpoint:', 'fcp:endpoint:', 'ag_ui:app:', 'agent:', or 'endpoint:' \
    prefixes are reserved for internal subsystems";

/// Policy: View sessions (read-only).
pub const SESSION_VIEW: Policy = Policy {
    id: "session.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSessionsManage)],
};

/// Policy: Manage sessions (create, update, delete).
pub const SESSION_MANAGE: Policy = Policy {
    id: "session.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSessionsManage)],
};

/// Optional, caller-supplied overrides applied when forking a session
/// (knowledge/runtime-resources/forking-sessions.md). Every field omitted (`None`) inherits the
/// parent session's value.
#[derive(Debug, Clone, Default)]
pub struct ForkOverrides {
    pub title: Option<String>,
    pub goal: Option<String>,
    pub tags: Option<Vec<String>>,
    pub model_id: Option<ModelId>,
    pub agent_id: Option<AgentId>,
    pub locale: Option<String>,
    pub system_prompt: Option<String>,
}

/// Session counts grouped by status.
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    pub total: u32,
    pub active: u32,
    pub idle: u32,
    pub started: u32,
    pub waiting_for_tool_results: u32,
}

pub struct SessionService {
    db: Arc<StorageBackend>,
    principal_service: PrincipalService,
    capability_registry: CapabilityRegistry,
    session_file_service: WorkspaceFileService,
    session_sandbox_service: Option<Arc<SessionSandboxService>>,
    caps: OrgCaps,
    resource_limits: ResourceLimitsConfig,
}

#[derive(Default)]
pub(crate) struct SessionListHydration {
    owners: HashMap<PrincipalId, PrincipalSummary>,
    effective_owners: HashMap<Uuid, PrincipalSummary>,
    agent_public_ids: HashMap<AgentId, AgentId>,
    agent_capability_ids: HashMap<AgentId, Vec<String>>,
    harness_capability_ids: HashMap<HarnessId, Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScopedMemoryContext {
    agent_id: Option<AgentId>,
    user_id: Option<Uuid>,
}

mod capabilities;
mod create;
mod fork;
mod lifecycle;
mod mounts;
mod query;

fn sanitize_session_capabilities(
    capabilities: Vec<AgentCapabilityConfig>,
) -> Vec<AgentCapabilityConfig> {
    capabilities
        .into_iter()
        .map(|mut capability| {
            if capability.capability_id() == SESSION_SANDBOX_CAPABILITY_ID
                && let Some(provider_config) = capability
                    .config_mut()
                    .get_mut("provider_config")
                    .and_then(serde_json::Value::as_object_mut)
            {
                let removed_api_base = provider_config.remove("api_base").is_some();
                let removed_toolbox_base = provider_config.remove("toolbox_base").is_some();
                if removed_api_base || removed_toolbox_base {
                    tracing::warn!(
                        "Ignoring session-level session_sandbox provider_config base URL overrides"
                    );
                }
            }
            capability
        })
        .collect()
}

fn memory_files_to_mount_entries(mut files: Vec<MemoryFileRow>) -> HashMap<String, MountEntry> {
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut entries = HashMap::new();

    for file in files {
        let path = file.path.trim_matches('/');
        if path.is_empty() {
            continue;
        }
        let segments = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if file.is_directory {
            insert_mount_directory(&mut entries, &segments);
            continue;
        }

        let bytes = file.content.unwrap_or_default();
        let (content, encoding) = SessionFile::encode_content(&bytes);
        insert_mount_file(&mut entries, &segments, content, encoding);
    }

    entries
}

fn ensure_no_reserved_memory_mounts(mounts: &[MountPoint]) -> Result<()> {
    for mount in mounts {
        if let Some(reserved) = reserved_memory_path(&mount.path) {
            return Err(BadRequestError::new(format!(
                "Mount path {} is reserved for server-managed memory ({reserved})",
                mount.path
            ))
            .into());
        }
    }
    Ok(())
}

fn ensure_no_reserved_memory_initial_files(files: &[InitialFile]) -> Result<()> {
    for file in files {
        if let Some(reserved) = reserved_memory_path(&file.path) {
            return Err(BadRequestError::new(format!(
                "Initial file path {} is reserved for server-managed memory ({reserved})",
                file.path
            ))
            .into());
        }
    }
    Ok(())
}

fn reserved_memory_path(path: &str) -> Option<&'static str> {
    let path = normalize_initial_file_path(path);
    if path == "/memory" || path.starts_with("/memory/") {
        Some("/memory/*")
    } else {
        None
    }
}

fn insert_mount_directory(entries: &mut HashMap<String, MountEntry>, segments: &[&str]) {
    let Some((name, rest)) = segments.split_first() else {
        return;
    };
    let entry = entries
        .entry((*name).to_string())
        .or_insert_with(|| MountEntry::directory(HashMap::new()));
    if !entry.source.is_directory() {
        *entry = MountEntry::directory(HashMap::new());
    }
    if let MountSource::InlineDirectory { entries } = &mut entry.source {
        insert_mount_directory(entries, rest);
    }
}

fn insert_mount_file(
    entries: &mut HashMap<String, MountEntry>,
    segments: &[&str],
    content: String,
    encoding: String,
) {
    let Some((name, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        entries.insert(
            (*name).to_string(),
            MountEntry::new(MountSource::InlineFile { content, encoding }),
        );
        return;
    }

    let entry = entries
        .entry((*name).to_string())
        .or_insert_with(|| MountEntry::directory(HashMap::new()));
    if !entry.source.is_directory() {
        *entry = MountEntry::directory(HashMap::new());
    }
    if let MountSource::InlineDirectory { entries } = &mut entry.source {
        insert_mount_file(entries, rest, content, encoding);
    }
}

// normalize_initial_file_path is imported from everruns_core::config_layer

#[cfg(test)]
mod tests_mounts_tests;
#[cfg(test)]
mod tests_sessions;
#[cfg(test)]
mod tests_support;
