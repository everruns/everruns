// Configurable, multi-scope Skills Capability (built on the session VFS)
//
// `SkillsCapability` (see `skills.rs`) scans a single VFS path
// (`/.agents/skills`) and exposes exactly `list_skills` + `activate_skill`.
// That is the right default for the hosted server, where the only skill
// source is the session virtual filesystem and extra skills arrive via the
// mount mechanism (`attach_skill.rs`).
//
// Embedders that want **multiple labeled skill sources** — for example a
// terminal coding agent with workspace, per-user "global", and binary-bundled
// "system" scopes — need three things this default does not provide: merged
// discovery across scopes with precedence, the ability to install/update a
// skill (`write_skill`), and a way to render `${SKILL_DIR}` in whatever
// namespace the embedder's shell actually runs in. `ScopedSkillsCapability`
// adds exactly those, while reusing the upstream `crate::skill` parser,
// validator, and substitution verbatim.
//
// SECURITY — sources are VFS-bound by construction. A scope is a *labeled VFS
// root* (e.g. `/.agents/skills`), never a host filesystem path. Every read,
// write, and discovery call goes through the injected `SessionFileSystem`
// (`ToolContext::file_store`), so the capability can never be pointed at the
// worker host: it can only ever see what that file store exposes. The embedder
// decides how each VFS root maps to storage (a sandboxed overlay on the server;
// a real on-disk directory in a single-user CLI), and that mapping lives behind
// the file store — it is not a configuration knob on this capability. There is
// deliberately no API here that accepts a host path.
//
// COMMAND-SUBSTITUTION GATE — unchanged from `skills.rs`. `!`cmd`` placeholders
// are never expanded; activating a skill never spawns a shell. Re-enabling that
// is gated upstream behind a non-user-spoofable provenance signal and a
// session-sandbox-backed executor (EVE-388 / TM-TOOL-020) and is intentionally
// out of scope here.

use super::{Capability, CapabilityStatus, SystemPromptContext};
use crate::skill::{
    SkillContext, expand_skill_arguments, parse_skill_md, substitute_activation_vars,
    validate_skill_name,
};
use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy};
use crate::tools::{Tool, ToolExecutionResult};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use everruns_core::{session_files::SessionFileSystem, tool_context::ToolContext};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::skills::SKILLS_CAPABILITY_ID;

/// Default VFS root for the workspace scope (matches `skills.rs`).
const DEFAULT_WORKSPACE_ROOT: &str = "/.agents/skills";
/// Workspace prefix for agent-facing display paths.
const WORKSPACE_PREFIX: &str = "/workspace";
/// Max skills listed in the system prompt (the rest reachable via `list_skills`).
const MAX_SKILLS_IN_PROMPT: usize = 15;
/// Max skill directories scanned per scope when building the system prompt.
const MAX_SKILLS_SCAN_PER_SCOPE: usize = 64;
/// Max description length in the system-prompt listing (truncated with "…").
const MAX_DESCRIPTION_CHARS: usize = 76;
/// Defensive cap for extra files installed alongside a skill.
const MAX_EXTRA_SKILL_FILES: usize = 64;
/// Defensive cap for one skill file body.
const MAX_SKILL_FILE_BYTES: usize = 1024 * 1024;
/// Registry kind used to make `activate_skill` idempotent within a session.
const SKILL_ACTIVATION_KIND: &str = "skill_activation";

fn skill_activation_resource_id(name: &str) -> String {
    format!("{SKILL_ACTIVATION_KIND}:{name}")
}

fn truncate_description(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated.trim_end())
}

// ============================================================================
// Scopes, resolver, and configuration
// ============================================================================

/// One labeled skill source: a VFS root plus whether the agent may write to it.
///
/// `vfs_root` is always a session-VFS path (e.g. `/.agents/skills`). It is never
/// a host filesystem path — see the module security note.
#[derive(Clone, Debug)]
pub struct SkillScope {
    /// Stable label surfaced to the model (e.g. `workspace`, `global`, `system`).
    pub label: String,
    /// VFS root scanned for `<name>/SKILL.md` skill directories.
    pub vfs_root: String,
    /// Whether `write_skill` may install/update skills in this scope.
    pub writable: bool,
}

impl SkillScope {
    pub fn new(label: impl Into<String>, vfs_root: impl Into<String>, writable: bool) -> Self {
        Self {
            label: label.into(),
            vfs_root: vfs_root.into(),
            writable,
        }
    }

    /// The VFS path of a skill directory under this scope.
    fn dir_vfs(&self, name: &str) -> String {
        format!("{}/{name}", self.vfs_root.trim_end_matches('/'))
    }

    /// The VFS path of a skill's `SKILL.md` under this scope.
    fn skill_md_vfs(&self, name: &str) -> String {
        format!("{}/SKILL.md", self.dir_vfs(name))
    }
}

/// Resolves the on-execution path of a skill directory.
///
/// `${SKILL_DIR}` (and the agent-facing display path) must be valid in the
/// namespace where the skill's commands and bundled-file reads actually run.
/// The default ([`VfsSkillDirResolver`]) keeps everything in the session VFS,
/// which is correct when the shell shares that namespace (the hosted server's
/// session sandbox). An embedder whose shell runs elsewhere — e.g. a CLI whose
/// `bash` runs on the host — implements this to return a path valid there.
pub trait SkillDirResolver: Send + Sync {
    /// Value substituted for `${SKILL_DIR}` during activation.
    fn skill_dir(&self, scope: &SkillScope, name: &str) -> String;

    /// Agent-facing display path for the skill directory (shown in
    /// `list_skills` / `read_skill`). Defaults to the workspace-prefixed VFS
    /// path, matching the `file_system` capability's display convention.
    fn display_dir(&self, scope: &SkillScope, name: &str) -> String {
        let vfs = scope.dir_vfs(name);
        if vfs.starts_with('/') {
            format!("{WORKSPACE_PREFIX}{vfs}")
        } else {
            format!("{WORKSPACE_PREFIX}/{vfs}")
        }
    }
}

/// Default resolver: `${SKILL_DIR}` and the display path stay in the VFS.
pub struct VfsSkillDirResolver;

impl SkillDirResolver for VfsSkillDirResolver {
    fn skill_dir(&self, scope: &SkillScope, name: &str) -> String {
        scope.dir_vfs(name)
    }
}

/// Configuration for [`ScopedSkillsCapability`].
#[derive(Clone)]
pub struct SkillsConfig {
    /// Skill scopes in precedence order, highest first. A skill directory name
    /// found in an earlier scope shadows the same name in a later one.
    pub scopes: Vec<SkillScope>,
    /// Resolver for `${SKILL_DIR}` and display paths.
    pub resolver: Arc<dyn SkillDirResolver>,
    /// When `true`, expose the `read_skill` and `write_skill` management tools.
    /// Off by default so the management surface is strictly opt-in.
    pub manage_tools: bool,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            scopes: vec![SkillScope::new("workspace", DEFAULT_WORKSPACE_ROOT, true)],
            resolver: Arc::new(VfsSkillDirResolver),
            manage_tools: false,
        }
    }
}

impl SkillsConfig {
    /// Locate the first writable scope matching `label` (or any writable scope
    /// when `label` is `None`).
    fn writable_scope(&self, label: Option<&str>) -> Option<&SkillScope> {
        self.scopes
            .iter()
            .find(|s| s.writable && label.is_none_or(|l| l == s.label))
    }

    fn scope_by_label(&self, label: &str) -> Option<&SkillScope> {
        self.scopes.iter().find(|s| s.label == label)
    }
}

// ============================================================================
// Capability
// ============================================================================

const SCOPED_SKILLS_SYSTEM_PROMPT: &str = "Skills are reusable instruction packs \
discovered across one or more scopes (e.g. your workspace, global config, and ones \
bundled with the agent). Use `list_skills` to see what's available and `activate_skill` \
(by name) to load one. Only activate skills relevant to the current task.";

/// Configurable, multi-scope skills capability. Discovers, activates, and
/// (optionally) manages skills across the configured scopes, entirely through
/// the session `SessionFileSystem`.
pub struct ScopedSkillsCapability {
    config: Arc<SkillsConfig>,
}

impl ScopedSkillsCapability {
    pub fn new(config: SkillsConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

#[async_trait]
impl Capability for ScopedSkillsCapability {
    fn id(&self) -> &str {
        SKILLS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Agent Skills"
    }

    fn description(&self) -> &str {
        "Discover, activate, and manage skills across multiple scopes (workspace, \
         global config, and bundled), all through the session filesystem."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("wand")
    }

    fn category(&self) -> Option<&str> {
        Some("Skills")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SCOPED_SKILLS_SYSTEM_PROMPT)
    }

    async fn system_prompt_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        let mut prompt = String::from(SCOPED_SKILLS_SYSTEM_PROMPT);

        if let Some(file_store) = ctx.file_store.as_ref() {
            let discovered = discover_skills(
                &self.config,
                file_store.as_ref(),
                ctx.session_id,
                Some(MAX_SKILLS_SCAN_PER_SCOPE),
            )
            .await;
            let visible: Vec<&DiscoveredSkill> = discovered
                .iter()
                .filter(|s| s.error.is_none() && !s.disable_model_invocation)
                .collect();

            if !visible.is_empty() {
                prompt.push_str("\n\nAvailable skills:\n");
                for skill in visible.iter().take(MAX_SKILLS_IN_PROMPT) {
                    let desc = truncate_description(&skill.description, MAX_DESCRIPTION_CHARS);
                    let invocable = if skill.user_invocable {
                        format!(" (/{})", skill.name)
                    } else {
                        String::new()
                    };
                    prompt.push_str(&format!(
                        "- **{}** [{}]: {}{}\n",
                        skill.name, skill.scope_label, desc, invocable
                    ));
                }
                if visible.len() > MAX_SKILLS_IN_PROMPT {
                    prompt.push_str(&format!(
                        "\n({} more — use `list_skills` to see all)\n",
                        visible.len() - MAX_SKILLS_IN_PROMPT
                    ));
                }
            }
        }

        Some(format!(
            "<capability id=\"{}\">\n{}\n</capability>",
            self.id(),
            prompt
        ))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        let mut tools: Vec<Box<dyn Tool>> = vec![
            Box::new(ListSkillsTool {
                config: self.config.clone(),
            }),
            Box::new(ActivateSkillTool {
                config: self.config.clone(),
            }),
        ];
        if self.config.manage_tools {
            tools.push(Box::new(ReadSkillTool {
                config: self.config.clone(),
            }));
            tools.push(Box::new(WriteSkillTool {
                config: self.config.clone(),
            }));
        }
        tools
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let readonly = ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true);
        let mut defs = vec![
            ToolDefinition::Builtin(BuiltinTool {
                name: "list_skills".to_string(),
                display_name: Some("List Skills".to_string()),
                description: "Discover available skills across all configured scopes.".to_string(),
                parameters: list_skills_schema(),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: readonly.clone(),
                full_parameters: None,
            }),
            ToolDefinition::Builtin(BuiltinTool {
                name: "activate_skill".to_string(),
                display_name: Some("Activate Skill".to_string()),
                description: "Activate a skill by name to load its full instructions.".to_string(),
                parameters: activate_skill_schema(),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: readonly.clone(),
                full_parameters: None,
            }),
        ];
        if self.config.manage_tools {
            defs.push(ToolDefinition::Builtin(BuiltinTool {
                name: "read_skill".to_string(),
                display_name: Some("Read Skill".to_string()),
                description: "Read one installed skill's SKILL.md and file manifest.".to_string(),
                parameters: read_skill_schema(),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: readonly,
                full_parameters: None,
            }));
            defs.push(ToolDefinition::Builtin(BuiltinTool {
                name: "write_skill".to_string(),
                display_name: Some("Write Skill".to_string()),
                description: "Install or update a skill in a writable scope.".to_string(),
                parameters: write_skill_schema(),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: ToolHints::default().with_idempotent(true),
                full_parameters: None,
            }));
        }
        defs
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }
}

// ============================================================================
// Discovery
// ============================================================================

/// One discovered skill after parsing its `SKILL.md`.
struct DiscoveredSkill {
    scope_label: String,
    scope_index: usize,
    dir_name: String,
    name: String,
    description: String,
    version: String,
    user_invocable: bool,
    disable_model_invocation: bool,
    error: Option<String>,
}

/// Discover every unique skill across scopes, in precedence order. The first
/// occurrence of a directory name wins; later (farther-scope) duplicates drop.
/// `scan_limit` bounds the number of skill directories read per scope. It is
/// `Some(MAX_SKILLS_SCAN_PER_SCOPE)` only when building the system prompt (to
/// bound per-turn reads); `list_skills` passes `None` so the full set is
/// returned, matching the existing `SkillsCapability` behavior.
async fn discover_skills(
    config: &SkillsConfig,
    fs: &dyn SessionFileSystem,
    session_id: SessionId,
    scan_limit: Option<usize>,
) -> Vec<DiscoveredSkill> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (scope_index, scope) in config.scopes.iter().enumerate() {
        let entries = match fs.list_directory(session_id, &scope.vfs_root).await {
            Ok(entries) => entries,
            Err(_) => continue, // absent scope is silently empty
        };
        let mut dirs: Vec<_> = entries.into_iter().filter(|e| e.is_directory).collect();
        dirs.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(limit) = scan_limit {
            dirs.truncate(limit);
        }
        for entry in dirs {
            if !seen.insert(entry.name.clone()) {
                continue;
            }
            let md_path = scope.skill_md_vfs(&entry.name);
            let content = match fs.read_file(session_id, &md_path).await {
                Ok(Some(file)) => file.content.unwrap_or_default(),
                _ => continue,
            };
            let discovered = match parse_skill_md(&content) {
                Ok(parsed) => DiscoveredSkill {
                    scope_label: scope.label.clone(),
                    scope_index,
                    dir_name: entry.name.clone(),
                    name: parsed.name,
                    description: parsed.description,
                    version: parsed.version,
                    user_invocable: parsed.user_invocable,
                    disable_model_invocation: parsed.disable_model_invocation,
                    error: None,
                },
                Err(errors) => DiscoveredSkill {
                    scope_label: scope.label.clone(),
                    scope_index,
                    name: entry.name.clone(),
                    description: String::new(),
                    version: String::new(),
                    user_invocable: false,
                    disable_model_invocation: false,
                    error: Some(format!("Invalid SKILL.md: {}", errors.join(", "))),
                    dir_name: entry.name.clone(),
                },
            };
            out.push(discovered);
        }
    }
    out
}

/// Locate the activatable skill `name`, honoring precedence. Returns the scope
/// index and the raw `SKILL.md` content.
async fn locate_skill(
    config: &SkillsConfig,
    fs: &dyn SessionFileSystem,
    session_id: SessionId,
    name: &str,
) -> Option<(usize, String)> {
    for (idx, scope) in config.scopes.iter().enumerate() {
        if let Ok(Some(file)) = fs.read_file(session_id, &scope.skill_md_vfs(name)).await {
            return Some((idx, file.content.unwrap_or_default()));
        }
    }
    None
}

// ============================================================================
// list_skills
// ============================================================================

fn list_skills_schema() -> Value {
    json!({ "type": "object", "properties": {}, "required": [] })
}

struct ListSkillsTool {
    config: Arc<SkillsConfig>,
}

#[async_trait]
impl Tool for ListSkillsTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        crate::tool_narration::narrate_skill(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    fn name(&self) -> &str {
        "list_skills"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Skills")
    }

    fn description(&self) -> &str {
        "Discover available skills across all configured scopes. Returns each \
         skill's name, description, and scope."
    }

    fn parameters_schema(&self) -> Value {
        list_skills_schema()
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("list_skills requires session context.")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(fs) = &ctx.file_store else {
            return ToolExecutionResult::tool_error(
                "File store not available. The session_file_system capability is required.",
            );
        };
        // `list_skills` is uncapped (scan_limit = None); the per-scope scan cap
        // applies only to system-prompt building.
        let discovered = discover_skills(&self.config, fs.as_ref(), ctx.session_id, None).await;
        let skills: Vec<Value> = discovered
            .into_iter()
            .map(|s| {
                let scope = &self.config.scopes[s.scope_index];
                let display = self.config.resolver.display_dir(scope, &s.dir_name);
                match s.error {
                    Some(error) => json!({
                        "name": s.dir_name,
                        "scope": s.scope_label,
                        "path": format!("{display}/SKILL.md"),
                        "error": error,
                    }),
                    None => json!({
                        "name": s.name,
                        "description": s.description,
                        "scope": s.scope_label,
                        "version": s.version,
                        "user_invocable": s.user_invocable,
                        "disable_model_invocation": s.disable_model_invocation,
                        "path": format!("{display}/SKILL.md"),
                    }),
                }
            })
            .collect();
        ToolExecutionResult::success(json!({ "count": skills.len(), "skills": skills }))
    }
}

// ============================================================================
// activate_skill
// ============================================================================

fn activate_skill_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "The skill directory name (e.g., 'pdf-processing')" },
            "arguments": { "type": "string", "description": "Optional arguments to pass to the skill for $ARGUMENTS substitution" }
        },
        "required": ["name"]
    })
}

struct ActivateSkillTool {
    config: Arc<SkillsConfig>,
}

#[async_trait]
impl Tool for ActivateSkillTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        crate::tool_narration::narrate_skill(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    fn name(&self) -> &str {
        "activate_skill"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Activate Skill")
    }

    fn description(&self) -> &str {
        "Activate a skill by name to load its full instructions. Skills resolve \
         across the configured scopes (earlier scopes win on conflict)."
    }

    fn parameters_schema(&self) -> Value {
        activate_skill_schema()
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("activate_skill requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(name) = arguments.get("name").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error(
                "Missing required parameter: name. Call as \
                 activate_skill({\"name\":\"ship\"}). Use list_skills to see valid skill names.",
            );
        };
        let skill_args = arguments
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if let Err(msg) = validate_requested_skill_name(name) {
            return ToolExecutionResult::tool_error(msg);
        }

        // Idempotence (EVE-337): re-activating the same skill within a session
        // re-emits the cached result with `already_active: true`.
        if let Some(registry) = &ctx.session_resource_registry {
            let resource_id = skill_activation_resource_id(name);
            if let Ok(Some(entry)) = registry.get(ctx.session_id, &resource_id).await
                && entry.status == crate::session_resource::SessionResourceStatus::Active
                && entry.kind == SKILL_ACTIVATION_KIND
                && let Value::Object(mut map) = entry.metadata
            {
                map.insert("already_active".to_string(), Value::Bool(true));
                return ToolExecutionResult::success(Value::Object(map));
            }
        }

        let Some(fs) = &ctx.file_store else {
            return ToolExecutionResult::tool_error(
                "File store not available. The session_file_system capability is required.",
            );
        };

        let Some((scope_index, content)) =
            locate_skill(&self.config, fs.as_ref(), ctx.session_id, name).await
        else {
            return ToolExecutionResult::tool_error(format!(
                "Skill '{name}' not found in any scope. Use list_skills to see what's available."
            ));
        };
        let scope = &self.config.scopes[scope_index];

        match parse_skill_md(&content) {
            Ok(parsed) => {
                // Substitution pipeline: arguments then activation vars. Command
                // injection (`!`cmd``) is intentionally NOT expanded — see the
                // module-level gate note.
                let expanded = expand_skill_arguments(&parsed.instructions, skill_args);
                let skill_dir = self.config.resolver.skill_dir(scope, name);
                let substituted =
                    substitute_activation_vars(&expanded, &ctx.session_id.to_string(), &skill_dir);
                let instructions = format!(
                    "<skill name=\"{}\">\n{}\n</skill>",
                    parsed.name, substituted
                );

                let mut result = json!({
                    "skill": parsed.name,
                    "scope": scope.label,
                    "description": parsed.description,
                    "instructions": instructions,
                });
                if parsed.context == SkillContext::Fork {
                    result["context"] = json!("fork");
                    result["agent"] = json!(parsed.agent.as_deref().unwrap_or("general-purpose"));
                    if let Some(model) = &parsed.model {
                        result["model"] = json!(model);
                    }
                }

                if let Some(registry) = &ctx.session_resource_registry {
                    let entry = crate::session_resource::RegisterSessionResource {
                        session_id: ctx.session_id,
                        resource_id: skill_activation_resource_id(name),
                        kind: SKILL_ACTIVATION_KIND.to_string(),
                        display_name: format!("skill:{name}"),
                        status: crate::session_resource::SessionResourceStatus::Active,
                        metadata: result.clone(),
                    };
                    if let Err(e) = registry.register(entry).await {
                        tracing::warn!(error = %e, skill = %parsed.name, "activate_skill: failed to record activation");
                    }
                }

                ToolExecutionResult::success(result)
            }
            Err(errors) => ToolExecutionResult::tool_error(format!(
                "Invalid SKILL.md for '{name}': {}",
                errors.join(", ")
            )),
        }
    }
}

// ============================================================================
// read_skill
// ============================================================================

fn read_skill_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "The skill directory name." },
            "scope": { "type": "string", "description": "Optional scope label to read from. Omit to use normal precedence." }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

struct ReadSkillTool {
    config: Arc<SkillsConfig>,
}

#[async_trait]
impl Tool for ReadSkillTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        crate::tool_narration::narrate_skill(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    fn name(&self) -> &str {
        "read_skill"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Skill")
    }

    fn description(&self) -> &str {
        "Read one installed skill's SKILL.md and file manifest. Use before \
         modifying or upgrading a skill."
    }

    fn parameters_schema(&self) -> Value {
        read_skill_schema()
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("read_skill requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(name) = arguments.get("name").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("Missing required parameter: name");
        };
        if let Err(msg) = validate_requested_skill_name(name) {
            return ToolExecutionResult::tool_error(msg);
        }
        let Some(fs) = &ctx.file_store else {
            return ToolExecutionResult::tool_error(
                "File store not available. The session_file_system capability is required.",
            );
        };

        let located = match arguments.get("scope").and_then(|v| v.as_str()) {
            Some(label) => {
                let Some(scope) = self.config.scope_by_label(label) else {
                    return ToolExecutionResult::tool_error(format!("Unknown scope '{label}'"));
                };
                let idx = self
                    .config
                    .scopes
                    .iter()
                    .position(|s| s.label == scope.label)
                    .unwrap();
                match fs
                    .read_file(ctx.session_id, &scope.skill_md_vfs(name))
                    .await
                {
                    Ok(Some(file)) => Some((idx, file.content.unwrap_or_default())),
                    _ => None,
                }
            }
            None => locate_skill(&self.config, fs.as_ref(), ctx.session_id, name).await,
        };

        let Some((scope_index, content)) = located else {
            return ToolExecutionResult::tool_error(format!(
                "Skill '{name}' not found. Use list_skills to see what's available."
            ));
        };
        let scope = &self.config.scopes[scope_index];
        let display = self.config.resolver.display_dir(scope, name);
        let files = skill_file_manifest(fs.as_ref(), ctx.session_id, &scope.dir_vfs(name)).await;

        ToolExecutionResult::success(json!({
            "name": name,
            "scope": scope.label,
            "path": format!("{display}/SKILL.md"),
            "skill_md": content,
            "files": files,
        }))
    }
}

/// Relative file manifest (excluding `SKILL.md`) for a skill directory, walked
/// through the VFS. Bounded by `MAX_EXTRA_SKILL_FILES`.
async fn skill_file_manifest(
    fs: &dyn SessionFileSystem,
    session_id: SessionId,
    dir_vfs: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![dir_vfs.to_string()];
    while let Some(dir) = queue.pop() {
        let Ok(entries) = fs.list_directory(session_id, &dir).await else {
            continue;
        };
        for entry in entries {
            if entry.is_directory {
                queue.push(entry.path.clone());
            } else {
                if let Some(rel) = entry.path.strip_prefix(&format!("{dir_vfs}/"))
                    && rel != "SKILL.md"
                {
                    out.push(rel.to_string());
                }
                if out.len() >= MAX_EXTRA_SKILL_FILES {
                    out.sort();
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

// ============================================================================
// write_skill
// ============================================================================

fn write_skill_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "scope": { "type": "string", "description": "Writable scope label to install into (e.g. 'workspace' or 'global'). Omit to use the first writable scope." },
            "name": { "type": "string", "description": "Skill directory name. Must match the SKILL.md frontmatter name." },
            "skill_md": { "type": "string", "description": "The complete SKILL.md contents, including frontmatter." },
            "files": {
                "type": "object",
                "description": "Optional bundled files keyed by relative path. Do not include SKILL.md here.",
                "additionalProperties": { "type": "string" }
            },
            "overwrite": { "type": "boolean", "description": "Whether to replace an existing skill. Defaults to true." }
        },
        "required": ["name", "skill_md"],
        "additionalProperties": false
    })
}

struct WriteSkillTool {
    config: Arc<SkillsConfig>,
}

#[async_trait]
impl Tool for WriteSkillTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        crate::tool_narration::narrate_skill(&tool_call.name, &tool_call.arguments, phase, locale)
    }

    fn name(&self) -> &str {
        "write_skill"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write Skill")
    }

    fn description(&self) -> &str {
        "Install or update a skill in a writable scope. Provide the full SKILL.md \
         and optional bundled files. Recreates skills from a registry/GitHub \
         source without requiring an installer."
    }

    fn parameters_schema(&self) -> Value {
        write_skill_schema()
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_idempotent(true)
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("write_skill requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        ctx: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(name) = arguments.get("name").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("'name' is required");
        };
        if let Err(msg) = validate_requested_skill_name(name) {
            return ToolExecutionResult::tool_error(msg);
        }
        let Some(skill_md) = arguments.get("skill_md").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::tool_error("'skill_md' is required");
        };
        if skill_md.len() > MAX_SKILL_FILE_BYTES {
            return ToolExecutionResult::tool_error(format!(
                "'skill_md' exceeds the {MAX_SKILL_FILE_BYTES} byte limit"
            ));
        }

        let parsed = match parse_skill_md(skill_md) {
            Ok(parsed) => parsed,
            Err(errors) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid SKILL.md: {}",
                    errors.join(", ")
                ));
            }
        };
        if parsed.name != name {
            return ToolExecutionResult::tool_error(format!(
                "'name' must match SKILL.md frontmatter name (got '{}')",
                parsed.name
            ));
        }

        let requested_label = arguments.get("scope").and_then(|v| v.as_str());
        // Reject a known-but-non-writable scope explicitly (e.g. a read-only
        // bundled "system" scope) rather than silently falling through.
        if let Some(label) = requested_label
            && let Some(scope) = self.config.scope_by_label(label)
            && !scope.writable
        {
            return ToolExecutionResult::tool_error(format!(
                "Scope '{label}' is read-only; choose a writable scope"
            ));
        }
        let Some(scope) = self.config.writable_scope(requested_label) else {
            return ToolExecutionResult::tool_error(match requested_label {
                Some(label) => format!("No writable scope named '{label}'"),
                None => "No writable skill scope is configured".to_string(),
            });
        };

        let overwrite = arguments
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let files = match collect_extra_files(arguments.get("files")) {
            Ok(files) => files,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let fs = match &ctx.file_store {
            Some(fs) => fs,
            None => {
                return ToolExecutionResult::tool_error(
                    "File store not available. The session_file_system capability is required.",
                );
            }
        };

        let md_path = scope.skill_md_vfs(name);
        if !overwrite && matches!(fs.read_file(ctx.session_id, &md_path).await, Ok(Some(_))) {
            return ToolExecutionResult::tool_error(format!(
                "Skill '{name}' already exists in '{}' scope; pass overwrite=true to update it",
                scope.label
            ));
        }

        if let Err(e) = fs
            .write_file(ctx.session_id, &md_path, skill_md, "text")
            .await
        {
            return ToolExecutionResult::tool_error(format!("could not write SKILL.md: {e}"));
        }
        for (rel, content) in &files {
            let target = format!("{}/{}", scope.dir_vfs(name), rel.display());
            if let Err(e) = fs
                .write_file(ctx.session_id, &target, content, "text")
                .await
            {
                return ToolExecutionResult::tool_error(format!(
                    "could not write '{}': {e}",
                    rel.display()
                ));
            }
        }

        let display = self.config.resolver.display_dir(scope, name);
        ToolExecutionResult::success(json!({
            "ok": true,
            "name": name,
            "scope": scope.label,
            "path": format!("{display}/SKILL.md"),
            "files_written": files.len() + 1,
            "message": "skill written; discoverable immediately via list_skills and activate_skill",
        }))
    }
}

// ============================================================================
// Validation helpers
// ============================================================================

fn validate_requested_skill_name(name: &str) -> Result<(), String> {
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(
            "Invalid skill name. Must be a simple directory name without path separators."
                .to_string(),
        );
    }
    validate_skill_name(name)
        .map_err(|errors| format!("Invalid skill name '{name}': {}", errors.join(", ")))
}

fn collect_extra_files(value: Option<&Value>) -> Result<Vec<(PathBuf, String)>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(map) = value.as_object() else {
        return Err("'files' must be an object of relative path to string content".to_string());
    };
    if map.len() > MAX_EXTRA_SKILL_FILES {
        return Err(format!(
            "'files' contains too many entries (max {MAX_EXTRA_SKILL_FILES})"
        ));
    }
    let mut out = Vec::with_capacity(map.len());
    for (raw_path, value) in map {
        let Some(content) = value.as_str() else {
            return Err(format!("file '{raw_path}' content must be a string"));
        };
        if content.len() > MAX_SKILL_FILE_BYTES {
            return Err(format!(
                "file '{raw_path}' exceeds the {MAX_SKILL_FILE_BYTES} byte limit"
            ));
        }
        out.push((validate_skill_file_path(raw_path)?, content.to_string()));
    }
    Ok(out)
}

/// Reject absolute paths, traversal, backslashes, and `SKILL.md` itself.
fn validate_skill_file_path(raw: &str) -> Result<PathBuf, String> {
    if raw.trim().is_empty() || raw.contains('\\') {
        return Err(format!("invalid skill file path '{raw}'"));
    }
    let path = PathBuf::from(raw);
    if path == Path::new("SKILL.md") || path.is_absolute() {
        return Err(format!("invalid skill file path '{raw}'"));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!("invalid skill file path '{raw}'"));
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
