// Skills Capability (built-in)
//
// Generic mechanism for filesystem-based skill discovery and activation.
// When enabled on an agent, provides:
// - System prompt explaining the skills system
// - `list_skills` tool: scans /.agents/skills/ in session VFS
// - `activate_skill` tool: loads SKILL.md instructions from VFS
//
// This is the built-in "skills" capability. It does NOT ship with any skills.
// Users upload SKILL.md files to /.agents/skills/{name}/SKILL.md in the
// session filesystem, and the agent discovers them at runtime.
//
// Database-registered skills are attached via AttachSkillCapability, which
// mounts skill files into the VFS so this capability discovers them.
//
// COMMAND-SUBSTITUTION TRUST GATE (see also `knowledge/project/skills-registry.md`
// "Activation Substitution Pipeline" and threat-model entry TM-TOOL-020):
// SKILL.md may contain ``!`command` `` placeholders that, when expanded by
// `preprocess_command_injections`, spawn a shell on the worker host. That is
// RCE if the SKILL.md was authored by an untrusted party. There is currently
// no platform-controlled provenance signal on `SessionFile` that proves a
// SKILL.md was placed by the capability/registry mount layer as opposed to by
// a user-facing path (session-files API, agent/session `initial_files`,
// runtime `write_file`). In particular, `SessionFile::is_readonly` is NOT
// such a signal: both the session-files API and `InitialFile` accept
// `is_readonly = true` from user input.
//
// Therefore `ActivateSkillFromVfsTool::execute_with_context` never invokes
// `preprocess_command_injections`; command placeholders remain literal. This
// preserves PR #1449's RCE fix while keeping the neutral parsing logic and its
// unit tests available for a follow-up once a non-user-spoofable provenance signal
// (e.g. a `mount_capability_id` column populated only by mount application
// code) is added to `SessionFile` / `SessionFileSystem`. See EVE-388.
//
// Re-enable must supply a session-sandbox-backed executor: execution MUST run against
// the bashkit shell (managed session sandbox) and the session virtual
// filesystem, not the worker host. Adding provenance alone would still be RCE
// against the worker. See threat-model TM-TOOL-020 step 6.

use super::{Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext};
use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use everruns_core::tool_context::ToolContext;
use serde_json::Value;

/// Skills capability ID (built-in)
pub const SKILLS_CAPABILITY_ID: &str = "skills";

/// Activate workspace skill discovery with its default configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skills;

impl everruns_capability::IntoCapability for Skills {
    fn into_capability(self) -> everruns_capability::CapabilitySpec {
        everruns_capability::CapabilityRef::new(SKILLS_CAPABILITY_ID).into()
    }
}

/// Path in session VFS where skills are discovered (reuse shared constant)
use super::attach_skill::SKILLS_DISCOVERY_PATH as SKILLS_PATH;

/// Kind tag used when registering activated skills in the session resource registry.
/// Idempotence for `activate_skill` (EVE-337) is implemented by recording each activation
/// under `resource_id = "skill_activation:{name}"` and reusing its metadata on re-entry.
const SKILL_ACTIVATION_KIND: &str = "skill_activation";

fn skill_activation_resource_id(name: &str) -> String {
    format!("{SKILL_ACTIVATION_KIND}:{name}")
}

/// Max skills to include in the system prompt (rest via list_skills tool)
const MAX_SKILLS_IN_PROMPT: usize = 15;

/// Max skill directories to scan when building the system prompt.
/// Bounds per-turn filesystem reads to avoid prompt-build DoS.
const MAX_SKILLS_SCAN_IN_PROMPT: usize = 64;

/// Max description length in system prompt (truncated with "…")
const MAX_DESCRIPTION_CHARS: usize = 76;

/// Workspace prefix for agent-facing paths (matches file_system capability convention)
const WORKSPACE_PREFIX: &str = "/workspace";

/// Truncate a string to `max_chars`, appending "…" if truncated.
/// Splits on the nearest char boundary at or before `max_chars`.
fn truncate_description(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated.trim_end())
}

/// Add workspace prefix for display paths shown to the agent
fn workspace_path(path: &str) -> String {
    if path.starts_with('/') {
        format!("{}{}", WORKSPACE_PREFIX, path)
    } else {
        format!("{}/{}", WORKSPACE_PREFIX, path)
    }
}

/// Built-in Skills Discovery Capability
///
/// Provides the generic skills mechanism. No skills are bundled — users upload
/// SKILL.md files to `/.agents/skills/{name}/SKILL.md` in the session VFS.
pub struct SkillsCapability;

/// Static skills system prompt (used by sync callers and as fallback)
const SKILLS_SYSTEM_PROMPT: &str = "Skills location: `/workspace/.agents/skills/{skill-name}/SKILL.md`. \
Only activate skills that are relevant to the current task.";

#[async_trait]
impl Capability for SkillsCapability {
    fn id(&self) -> &str {
        SKILLS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Agent Skills"
    }

    fn description(&self) -> &str {
        r#"Discover and activate skills from the session filesystem.

Skills are instruction packages (SKILL.md files) that teach the agent new abilities. Upload skills to `/workspace/.agents/skills/{name}/SKILL.md` and the agent will discover them automatically.

> [!TIP]
> Use the `list_skills` tool to see available skills, then `activate_skill` to load one."#
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Навички агента",
            r#"Виявляйте та активуйте навички з файлової системи сесії.

Навички — це пакети інструкцій (файли SKILL.md), які навчають агента нових умінь. Завантажте навички до `/workspace/.agents/skills/{name}/SKILL.md`, і агент виявить їх автоматично.

> [!TIP]
> Використовуйте інструмент `list_skills`, щоб переглянути доступні навички, а потім `activate_skill`, щоб завантажити потрібну."#,
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("wand")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SKILLS_SYSTEM_PROMPT)
    }

    /// Dynamically discovers skills from the session filesystem and includes
    /// them in the system prompt.
    ///
    /// When a file store is available, scans `/.agents/skills/` for SKILL.md
    /// files and lists discovered skills directly in the prompt. Falls back
    /// to the static prompt when no file store is available.
    async fn system_prompt_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        let file_store = match ctx.file_store.as_ref() {
            Some(fs) => fs,
            None => {
                // No file store — fall back to static prompt with capability wrapping
                return Some(format!(
                    "<capability id=\"{}\">\n{}\n</capability>",
                    self.id(),
                    SKILLS_SYSTEM_PROMPT
                ));
            }
        };

        // Scan /.agents/skills/ for SKILL.md files
        let entries = match file_store.list_directory(ctx.session_id, SKILLS_PATH).await {
            Ok(entries) => entries,
            Err(_) => {
                // Directory doesn't exist — use static prompt
                return Some(format!(
                    "<capability id=\"{}\">\n{}\n</capability>",
                    self.id(),
                    SKILLS_SYSTEM_PROMPT
                ));
            }
        };

        // Only scan a bounded number of skill directories for prompt generation.
        // Full discovery remains available through list_skills tool.
        let skill_dirs: Vec<_> = entries.iter().filter(|entry| entry.is_directory).collect();
        let scan_truncated = skill_dirs.len() > MAX_SKILLS_SCAN_IN_PROMPT;

        let mut discovered_skills = Vec::new();
        for entry in skill_dirs.iter().take(MAX_SKILLS_SCAN_IN_PROMPT) {
            let skill_md_path = format!("{}/SKILL.md", entry.path);
            if let Ok(Some(file)) = file_store.read_file(ctx.session_id, &skill_md_path).await {
                let content = file.content.as_deref().unwrap_or("");
                if let Ok(parsed) = crate::skill::parse_skill_md(content) {
                    discovered_skills.push((
                        parsed.name,
                        parsed.description,
                        parsed.user_invocable,
                        parsed.disable_model_invocation,
                    ));
                }
            }
        }

        let mut prompt = String::from(SKILLS_SYSTEM_PROMPT);

        if !discovered_skills.is_empty() {
            // Filter out skills where disable_model_invocation is true
            let model_visible_skills: Vec<_> = discovered_skills
                .iter()
                .filter(|(_, _, _, disable_model)| !disable_model)
                .collect();
            let total = model_visible_skills.len();
            if total > 0 {
                prompt.push_str("\n\nAvailable skills:\n");
            }
            for (name, description, user_invocable, _) in
                model_visible_skills.iter().take(MAX_SKILLS_IN_PROMPT)
            {
                let desc = truncate_description(description, MAX_DESCRIPTION_CHARS);
                let invocable_hint = if *user_invocable { " (/{name})" } else { "" };
                prompt.push_str(&format!("- **{name}**: {desc}{invocable_hint}\n"));
            }
            if total > MAX_SKILLS_IN_PROMPT {
                prompt.push_str(&format!(
                    "\n({} more skills available — use `list_skills` to see all)\n",
                    total - MAX_SKILLS_IN_PROMPT
                ));
            }
            if scan_truncated {
                prompt.push_str(
                    "\n(Additional skills may exist — use `list_skills` to view the full list)\n",
                );
            }
        }

        Some(format!(
            "<capability id=\"{}\">\n{}\n</capability>",
            self.id(),
            prompt
        ))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ListSkillsTool), Box::new(ActivateSkillFromVfsTool)]
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition::Builtin(BuiltinTool {
                name: "list_skills".to_string(),
                display_name: Some("List Skills".to_string()),
                description: "Discover available skills from the session filesystem. \
                    Scans /workspace/.agents/skills/ for SKILL.md files and returns their names \
                    and descriptions."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: ToolHints::default()
                    .with_readonly(true)
                    .with_idempotent(true),
                full_parameters: None,
            }),
            ToolDefinition::Builtin(BuiltinTool {
                name: "activate_skill".to_string(),
                display_name: Some("Activate Skill".to_string()),
                description: "Activate a skill by name to load its full instructions. \
                    The skill must exist at /workspace/.agents/skills/{name}/SKILL.md in the \
                    session filesystem."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The skill directory name (e.g., 'pdf-processing')"
                        },
                        "arguments": {
                            "type": "string",
                            "description": "Optional arguments to pass to the skill for $ARGUMENTS substitution"
                        }
                    },
                    "required": ["name"]
                }),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: ToolHints::default()
                    .with_readonly(true)
                    .with_idempotent(true),
                full_parameters: None,
            }),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }
}

// ============================================================================
// ListSkillsTool - Discovers skills from session VFS
// ============================================================================

/// Tool that scans `/.agents/skills/` in the session VFS for SKILL.md files.
#[derive(Debug)]
struct ListSkillsTool;

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
        "Discover available skills from the session filesystem."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
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
        ToolExecutionResult::tool_error(
            "list_skills requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let file_store = match &context.file_store {
            Some(fs) => fs,
            None => {
                return ToolExecutionResult::tool_error(
                    "File store not available. The session_file_system capability is required.",
                );
            }
        };

        // List directories under /.agents/skills/
        let entries = match file_store
            .list_directory(context.session_id, SKILLS_PATH)
            .await
        {
            Ok(entries) => entries,
            Err(_) => {
                // Directory doesn't exist yet — no skills available
                return ToolExecutionResult::success(serde_json::json!({
                    "skills": [],
                    "message": "No skills found. Upload skills to /workspace/.agents/skills/{name}/SKILL.md"
                }));
            }
        };

        let mut skills = Vec::new();

        for entry in &entries {
            if !entry.is_directory {
                continue;
            }

            let skill_md_path = format!("{}/SKILL.md", entry.path);
            if let Ok(Some(file)) = file_store
                .read_file(context.session_id, &skill_md_path)
                .await
            {
                let content = file.content.as_deref().unwrap_or("");
                match crate::skill::parse_skill_md(content) {
                    Ok(parsed) => {
                        skills.push(serde_json::json!({
                            "name": parsed.name,
                            "description": parsed.description,
                            "path": workspace_path(&skill_md_path),
                            "version": parsed.version,
                            "user_invocable": parsed.user_invocable,
                            "disable_model_invocation": parsed.disable_model_invocation,
                        }));
                    }
                    Err(errors) => {
                        skills.push(serde_json::json!({
                            "name": entry.name,
                            "path": workspace_path(&skill_md_path),
                            "error": format!("Invalid SKILL.md: {}", errors.join(", ")),
                        }));
                    }
                }
            }
        }

        ToolExecutionResult::success(serde_json::json!({
            "skills": skills,
            "count": skills.len(),
            "skills_path": workspace_path(SKILLS_PATH),
        }))
    }
}

// ============================================================================
// ActivateSkillFromVfsTool - Loads skill instructions from session VFS
// ============================================================================

/// Tool that reads a SKILL.md from `/.agents/skills/{name}/SKILL.md` and
/// returns its full instructions.
#[derive(Debug)]
struct ActivateSkillFromVfsTool;

#[async_trait]
impl Tool for ActivateSkillFromVfsTool {
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
        "Activate a skill by name to load its full instructions from the session filesystem."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill directory name (e.g., 'pdf-processing')"
                },
                "arguments": {
                    "type": "string",
                    "description": "Optional arguments to pass to the skill for $ARGUMENTS substitution"
                }
            },
            "required": ["name"]
        })
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
        ToolExecutionResult::tool_error(
            "activate_skill requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: name");
            }
        };

        let skill_args = arguments
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Validate name (prevent path traversal)
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return ToolExecutionResult::tool_error(
                "Invalid skill name. Must be a simple directory name without path separators.",
            );
        }

        // Validate name against the skill naming rules (lowercase letters, digits,
        // single hyphens, 1-64 chars). This also rejects `:` and any other
        // non-spec character so the session-resource key `skill_activation:{name}`
        // is unambiguous — see `skill_activation_resource_id`.
        if let Err(errors) = crate::skill::validate_skill_name(name) {
            return ToolExecutionResult::tool_error(format!(
                "Invalid skill name '{name}': {}",
                errors.join(", ")
            ));
        }

        // Idempotence (EVE-337): if this skill has already been activated in this
        // session, return the cached result with `already_active: true` instead of
        // replaying the full parse/expand/preprocess pipeline. Requires a session
        // resource registry; when no registry is wired in (unit tests, embedded
        // runtime without session state) we fall back to non-cached activation.
        if let Some(registry) = &context.session_resource_registry {
            let resource_id = skill_activation_resource_id(name);
            match registry.get(context.session_id, &resource_id).await {
                Ok(Some(entry))
                    if entry.status == crate::session_resource::SessionResourceStatus::Active =>
                {
                    if entry.kind != SKILL_ACTIVATION_KIND {
                        tracing::warn!(
                            skill = name,
                            resource_id = %resource_id,
                            entry_kind = %entry.kind,
                            expected_kind = SKILL_ACTIVATION_KIND,
                            "activate_skill: registry entry collides with unexpected kind; falling back to non-cached activation"
                        );
                    } else if let Value::Object(mut map) = entry.metadata {
                        map.insert("already_active".to_string(), Value::Bool(true));
                        return ToolExecutionResult::success(Value::Object(map));
                    } else {
                        tracing::warn!(
                            skill = name,
                            resource_id = %resource_id,
                            "activate_skill: cached entry has non-object metadata; falling back to non-cached activation"
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        skill = name,
                        "activate_skill: failed to read session resource registry; falling back to non-cached activation"
                    );
                }
            }
        }

        let file_store = match &context.file_store {
            Some(fs) => fs,
            None => {
                return ToolExecutionResult::tool_error(
                    "File store not available. The session_file_system capability is required.",
                );
            }
        };

        let skill_md_path = format!("{}/{}/SKILL.md", SKILLS_PATH, name);

        let file = match file_store
            .read_file(context.session_id, &skill_md_path)
            .await
        {
            Ok(Some(f)) => f,
            Ok(None) => {
                return ToolExecutionResult::tool_error(format!(
                    "Skill '{name}' not found at {}. \
                     Use list_skills to see available skills.",
                    workspace_path(&skill_md_path)
                ));
            }
            Err(e) => {
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to read skill file: {e}"
                ));
            }
        };

        // Parse the SKILL.md to extract instructions
        let content = file.content.as_deref().unwrap_or("");
        // Trust gate: `preprocess_command_injections` is gated OFF for every
        // source. `SessionFile::is_readonly` is user-settable via the
        // session-files API and `InitialFile`, so it cannot be used as the
        // trust signal. See module-level comment and EVE-388.
        //
        // `file` is still read above so future trust checks can look at
        // provenance metadata once it exists; for now the decision is fixed.
        let _ = &file;
        match crate::skill::parse_skill_md(content) {
            Ok(parsed) => {
                // Apply substitution pipeline:
                //   1. arguments ($ARGUMENTS, $N)
                //   2. env vars (${SESSION_ID}, ${SKILL_DIR})
                //   3. command injection (!`cmd`) — only for trusted sources
                let expanded =
                    crate::skill::expand_skill_arguments(&parsed.instructions, skill_args);
                let skill_dir = format!("{}/{}", SKILLS_PATH, name);
                let session_id_str = context.session_id.to_string();
                let substituted = crate::skill::substitute_activation_vars(
                    &expanded,
                    &session_id_str,
                    &skill_dir,
                );
                let preprocessed = substituted;
                let instructions = format!(
                    "<skill name=\"{}\">\n{}\n</skill>",
                    parsed.name, preprocessed
                );

                let mut result = serde_json::json!({
                    "skill": parsed.name,
                    "instructions": instructions,
                    "description": parsed.description,
                });

                // Include context and agent fields for fork-mode skills
                if parsed.context == crate::skill::SkillContext::Fork {
                    result["context"] = serde_json::json!("fork");
                    result["agent"] =
                        serde_json::json!(parsed.agent.as_deref().unwrap_or("general-purpose"));
                    if let Some(ref model) = parsed.model {
                        result["model"] = serde_json::json!(model);
                    }
                }

                // Register this activation so subsequent `activate_skill` calls for the
                // same skill in this session short-circuit (EVE-337). We store the full
                // tool result as metadata; the cache hit path re-emits it verbatim and
                // adds `already_active: true`.
                if let Some(registry) = &context.session_resource_registry {
                    // Key the registry entry on the tool-argument `name` (which is
                    // also the skill directory name and matches the cache-lookup key
                    // above). Using `parsed.name` here would diverge if the
                    // frontmatter `name` ever drifts from the directory, breaking
                    // the idempotence contract on the second activation.
                    let entry = crate::session_resource::RegisterSessionResource {
                        session_id: context.session_id,
                        resource_id: skill_activation_resource_id(name),
                        kind: SKILL_ACTIVATION_KIND.to_string(),
                        display_name: format!("skill:{name}"),
                        status: crate::session_resource::SessionResourceStatus::Active,
                        metadata: result.clone(),
                    };
                    if let Err(e) = registry.register(entry).await {
                        tracing::warn!(
                            error = %e,
                            skill = %parsed.name,
                            "activate_skill: failed to record activation in session resource registry; skill still returned but re-activation will replay"
                        );
                    }
                }

                ToolExecutionResult::success(result)
            }
            Err(errors) => ToolExecutionResult::tool_error(format!(
                "Invalid SKILL.md at {}: {}",
                workspace_path(&skill_md_path),
                errors.join(", ")
            )),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Capability;
    use crate::error::Result;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::session_resource::{
        RegisterSessionResource, SessionResourceEntry, SessionResourceFilter, SessionResourceStatus,
    };
    use crate::typed_id::SessionId;
    use everruns_core::session_files::SessionFileSystem;
    use everruns_core::session_services::SessionResourceRegistry;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct FileSystemDependencyFixture;

    impl Capability for FileSystemDependencyFixture {
        fn id(&self) -> &str {
            "session_file_system"
        }
        fn name(&self) -> &str {
            "Fixture Filesystem"
        }
        fn description(&self) -> &str {
            "Host-supplied filesystem dependency fixture."
        }
    }

    // ========================================================================
    // MockFileStore for testing skill discovery
    // ========================================================================

    /// In-memory file store supporting both files and directories.
    struct MockFileStore {
        /// Files: (session_id, path) -> content
        files: Mutex<HashMap<(SessionId, String), String>>,
        /// Readonly flag per (session_id, path). Absent = writable.
        readonly_files: Mutex<std::collections::HashSet<(SessionId, String)>>,
        /// Directories: (session_id, path)
        dirs: Mutex<std::collections::HashSet<(SessionId, String)>>,
        /// Counter used by the idempotence test to prove the cache-hit path
        /// does not re-read SKILL.md on the second activation.
        read_count: AtomicUsize,
    }

    impl MockFileStore {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
                readonly_files: Mutex::new(std::collections::HashSet::new()),
                dirs: Mutex::new(std::collections::HashSet::new()),
                read_count: AtomicUsize::new(0),
            }
        }

        fn read_count(&self) -> usize {
            self.read_count.load(Ordering::SeqCst)
        }

        /// Add a file and auto-create parent directories
        fn add_file(&self, session_id: SessionId, path: &str, content: &str) {
            self.files
                .lock()
                .unwrap()
                .insert((session_id, path.to_string()), content.to_string());

            // Auto-create parent directories
            let mut dir = path.to_string();
            while let Some(idx) = dir.rfind('/') {
                if idx == 0 {
                    break;
                }
                dir = dir[..idx].to_string();
                self.dirs.lock().unwrap().insert((session_id, dir.clone()));
            }
        }

        /// Add a file and mark it as readonly (simulates a capability or
        /// registry mount).
        fn add_readonly_file(&self, session_id: SessionId, path: &str, content: &str) {
            self.add_file(session_id, path, content);
            self.readonly_files
                .lock()
                .unwrap()
                .insert((session_id, path.to_string()));
        }
    }

    #[async_trait]
    impl SessionFileSystem for MockFileStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> Result<Option<SessionFile>> {
            self.read_count.fetch_add(1, Ordering::SeqCst);
            let files = self.files.lock().unwrap();
            let readonly_files = self.readonly_files.lock().unwrap();
            if let Some(content) = files.get(&(session_id, path.to_string())) {
                let is_readonly = readonly_files.contains(&(session_id, path.to_string()));
                Ok(Some(SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.to_string(),
                    name: path.split('/').next_back().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly,
                    content: Some(content.clone()),
                    encoding: "text".to_string(),
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            _encoding: &str,
        ) -> Result<SessionFile> {
            self.add_file(session_id, path, content);
            Ok(SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.to_string(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: false,
                is_readonly: false,
                content: Some(content.to_string()),
                encoding: "text".to_string(),
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            Ok(false)
        }

        async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
            let files = self.files.lock().unwrap();
            let dirs = self.dirs.lock().unwrap();
            let mut entries = Vec::new();
            let mut seen_dirs = std::collections::HashSet::new();

            let prefix = if path.ends_with('/') {
                path.to_string()
            } else {
                format!("{}/", path)
            };

            // Find files directly under this path
            for ((sid, file_path), content) in files.iter() {
                if *sid != session_id || !file_path.starts_with(&prefix) {
                    continue;
                }
                let remainder = &file_path[prefix.len()..];
                if !remainder.contains('/') {
                    entries.push(FileInfo {
                        id: uuid::Uuid::new_v4(),
                        session_id: session_id.into(),
                        path: file_path.clone(),
                        name: remainder.to_string(),
                        is_directory: false,
                        is_readonly: false,
                        size_bytes: content.len() as i64,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    });
                }
            }

            // Find subdirectories directly under this path
            for (sid, dir_path) in dirs.iter() {
                if *sid != session_id || !dir_path.starts_with(&prefix) {
                    continue;
                }
                let remainder = &dir_path[prefix.len()..];
                // Only direct children (no nested slashes)
                if !remainder.contains('/')
                    && !remainder.is_empty()
                    && seen_dirs.insert(dir_path.clone())
                {
                    entries.push(FileInfo {
                        id: uuid::Uuid::new_v4(),
                        session_id: session_id.into(),
                        path: dir_path.clone(),
                        name: remainder.to_string(),
                        is_directory: true,
                        is_readonly: false,
                        size_bytes: 0,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    });
                }
            }

            Ok(entries)
        }

        async fn stat_file(&self, _session_id: SessionId, _path: &str) -> Result<Option<FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
            self.dirs
                .lock()
                .unwrap()
                .insert((session_id, path.to_string()));
            Ok(FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.to_string(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    fn valid_skill_md(name: &str, desc: &str) -> String {
        format!("---\nname: {name}\ndescription: {desc}\n---\n\n# Instructions\nDo the thing.")
    }

    fn make_context(file_store: Arc<MockFileStore>) -> ToolContext {
        ToolContext::with_file_store(SessionId::new(), file_store)
    }

    // ========================================================================
    // TestSessionResourceRegistry — in-memory registry for the idempotence test
    // ========================================================================

    #[derive(Default)]
    struct TestSessionResourceRegistry {
        entries: Mutex<HashMap<String, SessionResourceEntry>>,
    }

    #[async_trait]
    impl SessionResourceRegistry for TestSessionResourceRegistry {
        async fn register(&self, entry: RegisterSessionResource) -> Result<SessionResourceEntry> {
            let stored = SessionResourceEntry {
                resource_id: entry.resource_id.clone(),
                session_id: entry.session_id,
                kind: entry.kind,
                display_name: entry.display_name,
                status: entry.status,
                metadata: entry.metadata,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            self.entries
                .lock()
                .unwrap()
                .insert(entry.resource_id, stored.clone());
            Ok(stored)
        }

        async fn update_status(
            &self,
            _session_id: SessionId,
            resource_id: &str,
            status: SessionResourceStatus,
        ) -> Result<Option<SessionResourceEntry>> {
            let mut entries = self.entries.lock().unwrap();
            if let Some(entry) = entries.get_mut(resource_id) {
                entry.status = status;
                entry.updated_at = chrono::Utc::now();
                return Ok(Some(entry.clone()));
            }
            Ok(None)
        }

        async fn get(
            &self,
            _session_id: SessionId,
            resource_id: &str,
        ) -> Result<Option<SessionResourceEntry>> {
            Ok(self.entries.lock().unwrap().get(resource_id).cloned())
        }

        async fn list(
            &self,
            _session_id: SessionId,
            _filter: Option<&SessionResourceFilter>,
        ) -> Result<Vec<SessionResourceEntry>> {
            Ok(self.entries.lock().unwrap().values().cloned().collect())
        }

        async fn deregister(&self, _session_id: SessionId, resource_id: &str) -> Result<bool> {
            Ok(self.entries.lock().unwrap().remove(resource_id).is_some())
        }
    }

    // ========================================================================
    // Capability Trait Tests
    // ========================================================================

    // Metadata, tool-list, dependency, and builtin-registration constants are
    // covered registry-wide by `builtin_capabilities_satisfy_registry_invariants`
    // in `capabilities::tests`; the per-capability constant mirrors were removed.

    #[test]
    fn test_skills_has_system_prompt() {
        let cap = SkillsCapability;
        let prompt = cap.system_prompt_addition().unwrap();

        assert!(prompt.contains("/workspace/.agents/skills/"));
    }

    // ========================================================================
    // Tool Basics (no context)
    // ========================================================================

    #[test]
    fn test_list_skills_requires_context() {
        let tool = ListSkillsTool;
        assert!(tool.requires_context());
    }

    #[test]
    fn test_activate_skill_requires_context() {
        let tool = ActivateSkillFromVfsTool;
        assert!(tool.requires_context());
    }

    #[tokio::test]
    async fn test_list_skills_without_context() {
        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_error());
    }

    #[tokio::test]
    async fn test_activate_skill_without_context() {
        let tool = ActivateSkillFromVfsTool;
        let result = tool.execute(serde_json::json!({"name": "test"})).await;
        assert!(result.is_error());
    }

    // ========================================================================
    // Tool Error Cases
    // ========================================================================

    #[tokio::test]
    async fn test_activate_skill_missing_name() {
        let tool = ActivateSkillFromVfsTool;
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Missing required parameter"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_path_traversal_blocked() {
        let tool = ActivateSkillFromVfsTool;
        let context = ToolContext::new(SessionId::new());

        // ".." traversal
        let result = tool
            .execute_with_context(serde_json::json!({"name": "../etc/passwd"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid skill name")),
            other => panic!("Expected ToolError, got: {:?}", other),
        }

        // Forward slash
        let result = tool
            .execute_with_context(serde_json::json!({"name": "foo/bar"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid skill name")),
            other => panic!("Expected ToolError, got: {:?}", other),
        }

        // Backslash
        let result = tool
            .execute_with_context(serde_json::json!({"name": "foo\\bar"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid skill name")),
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    // Regression for EVE-337 review: names are validated against the skill
    // naming rules before being used as part of the `skill_activation:{name}`
    // resource-registry key. Reject the delimiter character explicitly.
    #[tokio::test]
    async fn test_activate_skill_rejects_resource_id_delimiter() {
        let tool = ActivateSkillFromVfsTool;
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(
                serde_json::json!({"name": "skill_activation:evil"}),
                &context,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Invalid skill name"), "got: {msg}");
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_no_file_store() {
        let tool = ListSkillsTool;
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("File store not available"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_no_file_store() {
        let tool = ActivateSkillFromVfsTool;
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(serde_json::json!({"name": "test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("File store not available"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    // ========================================================================
    // list_skills with MockFileStore
    // ========================================================================

    #[tokio::test]
    async fn test_list_skills_empty_directory() {
        let fs = Arc::new(MockFileStore::new());
        let context = make_context(fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert!(skills.is_empty());
                assert_eq!(val["count"], 0);
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_discovers_valid_skill() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/pdf-tool/SKILL.md",
            &valid_skill_md("pdf-tool", "Extract text from PDFs"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0]["name"], "pdf-tool");
                assert_eq!(skills[0]["description"], "Extract text from PDFs");
                assert_eq!(val["count"], 1);
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_discovers_multiple_skills() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/pdf-tool/SKILL.md",
            &valid_skill_md("pdf-tool", "Extract text from PDFs"),
        );
        fs.add_file(
            session_id,
            "/.agents/skills/data-analysis/SKILL.md",
            &valid_skill_md("data-analysis", "Analyze datasets"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 2);
                assert_eq!(val["count"], 2);
                let names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
                assert!(names.contains(&"pdf-tool"));
                assert!(names.contains(&"data-analysis"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_reports_invalid_skill_md() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/bad-skill/SKILL.md",
            "not valid frontmatter",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                // Should report the error but still include the entry
                assert!(
                    skills[0]["error"]
                        .as_str()
                        .unwrap()
                        .contains("Invalid SKILL.md")
                );
                assert_eq!(skills[0]["name"], "bad-skill");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // ========================================================================
    // activate_skill with MockFileStore
    // ========================================================================

    #[tokio::test]
    async fn test_activate_skill_success() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/pdf-tool/SKILL.md",
            &valid_skill_md("pdf-tool", "Extract text from PDFs"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "pdf-tool"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "pdf-tool");
                assert_eq!(val["description"], "Extract text from PDFs");
                let instructions = val["instructions"].as_str().unwrap();
                assert!(instructions.contains("<skill name=\"pdf-tool\">"));
                assert!(instructions.contains("# Instructions"));
                assert!(instructions.contains("</skill>"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // Regression test for EVE-337: activating the same skill twice within a
    // session must short-circuit on the second call (cached handle, no VFS
    // re-read) and annotate the response with `already_active: true`.
    #[tokio::test]
    async fn test_activate_skill_is_idempotent_within_session() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/pdf-tool/SKILL.md",
            &valid_skill_md("pdf-tool", "Extract text from PDFs"),
        );

        let registry: Arc<dyn SessionResourceRegistry> =
            Arc::new(TestSessionResourceRegistry::default());
        let context = ToolContext::with_file_store(session_id, fs.clone())
            .with_session_resource_registry(registry.clone());
        let tool = ActivateSkillFromVfsTool;

        let first = tool
            .execute_with_context(serde_json::json!({"name": "pdf-tool"}), &context)
            .await;
        let first_val = match first {
            ToolExecutionResult::Success(val) => val,
            other => panic!("Expected Success, got: {:?}", other),
        };
        assert_eq!(first_val["skill"], "pdf-tool");
        assert!(
            first_val.get("already_active").is_none(),
            "first activation must not carry already_active"
        );
        let reads_after_first = fs.read_count();
        assert!(reads_after_first >= 1, "first call must read SKILL.md");

        let second = tool
            .execute_with_context(serde_json::json!({"name": "pdf-tool"}), &context)
            .await;
        let second_val = match second {
            ToolExecutionResult::Success(val) => val,
            other => panic!("Expected Success, got: {:?}", other),
        };
        assert_eq!(second_val["already_active"], serde_json::Value::Bool(true));
        assert_eq!(second_val["skill"], first_val["skill"]);
        assert_eq!(second_val["description"], first_val["description"]);
        assert_eq!(second_val["instructions"], first_val["instructions"]);
        assert_eq!(
            fs.read_count(),
            reads_after_first,
            "cache hit must not re-read SKILL.md from the VFS"
        );

        // Registry holds exactly one entry, under the documented resource_id.
        let entry = registry
            .get(session_id, "skill_activation:pdf-tool")
            .await
            .unwrap()
            .expect("registry should contain the activation entry");
        assert_eq!(entry.kind, "skill_activation");
        assert_eq!(entry.status, SessionResourceStatus::Active);
    }

    #[tokio::test]
    async fn test_activate_skill_not_found() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "nonexistent"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("not found"));
                assert!(msg.contains("nonexistent"));
                assert!(msg.contains("list_skills"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_invalid_skill_md() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/bad-skill/SKILL.md",
            "no frontmatter here",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "bad-skill"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Invalid SKILL.md"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_with_context_fork() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/research/SKILL.md",
            "---\nname: research\ndescription: Deep research.\ncontext: fork\nagent: Explore\n---\n\nResearch the topic.",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "research"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "research");
                assert_eq!(val["context"], "fork");
                assert_eq!(val["agent"], "Explore");
                // Instructions are still included (caller decides how to use them)
                let instructions = val["instructions"].as_str().unwrap();
                assert!(instructions.contains("Research the topic"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_fork_default_agent() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/analyze/SKILL.md",
            "---\nname: analyze\ndescription: Analyze code.\ncontext: fork\n---\n\nAnalyze the code.",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "analyze"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["context"], "fork");
                assert_eq!(val["agent"], "general-purpose");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_inline_no_context_field() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/inline-skill/SKILL.md",
            &valid_skill_md("inline-skill", "An inline skill"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "inline-skill"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "inline-skill");
                // No context or agent fields for inline skills
                assert!(val.get("context").is_none());
                assert!(val.get("agent").is_none());
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_fork_with_model() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/quick-lint/SKILL.md",
            "---\nname: quick-lint\ndescription: Fast lint.\ncontext: fork\nmodel: claude-haiku-4-5-20251001\n---\n\nLint check.",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "quick-lint"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["context"], "fork");
                assert_eq!(val["agent"], "general-purpose");
                assert_eq!(val["model"], "claude-haiku-4-5-20251001");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_inline_no_model_in_result() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/my-skill/SKILL.md",
            "---\nname: my-skill\ndescription: A skill.\nmodel: gpt-4o\n---\n\nBody.",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "my-skill"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                // Inline skill: no model field in result (model only applies with fork)
                assert!(val.get("context").is_none());
                assert!(val.get("model").is_none());
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // ========================================================================
    // Integration: CapabilityInfo DTO
    // ========================================================================

    #[test]
    fn test_capability_info_from_core_marks_is_skill() {
        use crate::capability_dto::CapabilityInfo;
        let cap = SkillsCapability;
        let info = CapabilityInfo::from_core(&cap);

        assert_eq!(info.id.as_str(), "skills");
        assert!(info.is_skill, "skills capability should have is_skill=true");
        assert!(!info.is_mcp);
        assert_eq!(info.category, Some("Core".to_string()));
        assert!(!info.tool_definitions.is_empty());
        assert!(!info.dependencies.is_empty());
    }

    // ========================================================================
    // Integration: Dependency Resolution
    // ========================================================================

    #[test]
    fn test_skills_dependency_resolution() {
        use crate::capabilities::resolve_dependencies;

        let mut registry = crate::capabilities::CapabilityRegistry::new();
        crate::register_runtime_capabilities(&mut registry).unwrap();
        registry.register(FileSystemDependencyFixture);
        let resolved = resolve_dependencies(&["skills".to_string()], &registry).unwrap();

        // Should auto-include session_file_system as dependency
        assert!(
            resolved
                .resolved_ids
                .contains(&"session_file_system".to_string()),
            "skills should pull in session_file_system dependency"
        );
        assert!(resolved.resolved_ids.contains(&"skills".to_string()));
        assert!(
            resolved
                .added_as_dependencies
                .contains(&"session_file_system".to_string()),
            "session_file_system should be marked as auto-added"
        );
    }

    // ========================================================================
    // Integration: apply_capabilities
    // ========================================================================

    #[tokio::test]
    async fn test_apply_capabilities_with_skills() {
        use crate::capabilities::SystemPromptContext;
        use crate::runtime_agent::RuntimeAgentBuilder;

        let mut registry = crate::capabilities::CapabilityRegistry::new();
        crate::register_runtime_capabilities(&mut registry).unwrap();
        registry.register(FileSystemDependencyFixture);
        let ctx = SystemPromptContext::without_file_store(crate::typed_id::SessionId::new());
        // Use builder pattern which resolves dependencies automatically
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["skills".to_string()], &registry, &ctx)
            .await
            .model("gpt-5.2")
            .build();

        // System prompt should include skills capability section
        assert!(
            runtime_agent
                .system_prompt
                .contains("/workspace/.agents/skills/"),
            "System prompt should mention skills path"
        );
        assert!(
            runtime_agent
                .system_prompt
                .contains("/workspace/.agents/skills/"),
            "System prompt should mention skills path with workspace prefix"
        );
        assert!(
            runtime_agent
                .system_prompt
                .contains("<capability id=\"skills\">"),
            "Should include skills capability in XML tags"
        );

        // The filesystem implementation is host-owned; this core fixture only
        // proves the dependency participates in composition.
        let tool_names: Vec<&str> = runtime_agent.tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"list_skills"));
        assert!(tool_names.contains(&"activate_skill"));
    }

    // ========================================================================
    // Dynamic system_prompt_contribution tests
    // ========================================================================

    #[tokio::test]
    async fn test_contribution_includes_discovered_skills() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        store.add_file(
            session_id,
            "/.agents/skills/pdf-processor/SKILL.md",
            &valid_skill_md("pdf-processor", "Process PDF files"),
        );
        store.add_file(
            session_id,
            "/.agents/skills/data-analysis/SKILL.md",
            &valid_skill_md("data-analysis", "Analyze datasets"),
        );

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
            model: None,
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("pdf-processor"));
        assert!(result.contains("data-analysis"));
        assert!(result.contains("Available skills:"));
    }

    #[tokio::test]
    async fn test_contribution_static_when_no_file_store() {
        let cap = SkillsCapability;
        let ctx = SystemPromptContext::without_file_store(SessionId::new());

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("/workspace/.agents/skills/"));
        // No "Available skills:" section
        assert!(!result.contains("Available skills:"));
    }

    #[tokio::test]
    async fn test_contribution_static_when_no_skills_dir() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());

        let ctx = SystemPromptContext {
            session_id: SessionId::new(),
            locale: None,
            file_store: Some(store),
            model: None,
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("/workspace/.agents/skills/"));
        // No "Available skills:" section (dir doesn't exist)
        assert!(!result.contains("Available skills:"));
    }

    // ========================================================================
    // truncate_description tests
    // ========================================================================

    #[test]
    fn test_truncate_short_description() {
        assert_eq!(truncate_description("Short desc", 76), "Short desc");
    }

    #[test]
    fn test_truncate_exact_limit() {
        let s = "a".repeat(76);
        assert_eq!(truncate_description(&s, 76), s);
    }

    #[test]
    fn test_truncate_long_description() {
        let s = "a".repeat(100);
        let result = truncate_description(&s, 76);
        assert!(result.ends_with('…'));
        // 75 chars + "…" = 76 display chars
        assert_eq!(result.chars().count(), 76);
    }

    #[test]
    fn test_truncate_preserves_words_trimming() {
        let s = "Extract text and tables from PDF files, fill forms, merge documents, and do other cool things too";
        let result = truncate_description(s, 76);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 76);
    }

    // ========================================================================
    // System prompt caps at MAX_SKILLS_IN_PROMPT
    // ========================================================================

    #[tokio::test]
    async fn test_contribution_caps_at_max_skills() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Add 20 skills (exceeds MAX_SKILLS_IN_PROMPT = 15)
        for i in 0..20 {
            let name = format!("skill-{:02}", i);
            store.add_file(
                session_id,
                &format!("/.agents/skills/{}/SKILL.md", name),
                &valid_skill_md(&name, &format!("Description for skill {}", i)),
            );
        }

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
            model: None,
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();

        // Should contain "Available skills:"
        assert!(result.contains("Available skills:"));

        // Count how many skill entries appear (lines starting with "- **skill-")
        let skill_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.starts_with("- **skill-"))
            .collect();
        assert_eq!(skill_lines.len(), MAX_SKILLS_IN_PROMPT);

        // Should contain overflow message
        assert!(result.contains("5 more skills available"));
        assert!(result.contains("list_skills"));
    }

    #[tokio::test]
    async fn test_contribution_no_overflow_at_limit() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Add exactly MAX_SKILLS_IN_PROMPT skills
        for i in 0..MAX_SKILLS_IN_PROMPT {
            let name = format!("skill-{:02}", i);
            store.add_file(
                session_id,
                &format!("/.agents/skills/{}/SKILL.md", name),
                &valid_skill_md(&name, &format!("Description for skill {}", i)),
            );
        }

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
            model: None,
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();

        let skill_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.starts_with("- **skill-"))
            .collect();
        assert_eq!(skill_lines.len(), MAX_SKILLS_IN_PROMPT);

        // No overflow message
        assert!(!result.contains("more skills available"));
    }

    #[tokio::test]
    async fn test_contribution_limits_skill_scan_reads() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        for i in 0..(MAX_SKILLS_SCAN_IN_PROMPT + 20) {
            let name = format!("scan-skill-{:03}", i);
            store.add_file(
                session_id,
                &format!("/.agents/skills/{name}/SKILL.md"),
                &valid_skill_md(&name, "Scan limit test"),
            );
        }

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store.clone()),
            model: None,
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();

        assert_eq!(store.read_count(), MAX_SKILLS_SCAN_IN_PROMPT);
        assert!(result.contains("Additional skills may exist"));
    }

    // ========================================================================
    // Integration: AttachSkillCapability mount → SkillsCapability discovery
    // ========================================================================

    /// Simulate the runtime flow: AttachSkillCapability produces a mount,
    /// we materialize it into MockFileStore, then SkillsCapability discovers it.
    fn materialize_mount_into_store(
        store: &MockFileStore,
        session_id: SessionId,
        mount: &crate::capability_types::MountPoint,
    ) {
        use crate::capability_types::MountSource;
        fn walk(store: &MockFileStore, session_id: SessionId, base: &str, source: &MountSource) {
            match source {
                MountSource::InlineFile { content, .. } => {
                    store.add_file(session_id, base, content);
                }
                MountSource::InlineDirectory { entries } => {
                    for (name, entry) in entries {
                        let path = format!("{}/{}", base, name);
                        walk(store, session_id, &path, &entry.source);
                    }
                }
                MountSource::Virtual { .. } => {
                    // Virtual mounts are not materialized in tests
                }
            }
        }
        walk(store, session_id, &mount.path, &mount.source);
    }

    #[tokio::test]
    async fn test_attach_skill_mount_discovered_by_list_skills() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "pdf-tool".to_string(),
            "Extract text from PDFs".to_string(),
            "# Instructions\nUse pdfplumber to extract.".to_string(),
            vec![],
        );

        // Materialize mount into VFS mock
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // list_skills should discover it
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0]["name"], "pdf-tool");
                assert_eq!(skills[0]["description"], "Extract text from PDFs");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_attach_skill_mount_activatable_by_skills_capability() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "code-review".to_string(),
            "Review code for issues".to_string(),
            "# Instructions\nReview the code carefully.\n\n## Steps\n1. Check style\n2. Check logic"
                .to_string(),
            vec![],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // activate_skill should load instructions
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ActivateSkillFromVfsTool;
        let result = tool
            .execute_with_context(serde_json::json!({"name": "code-review"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "code-review");
                assert_eq!(val["description"], "Review code for issues");
                let instructions = val["instructions"].as_str().unwrap();
                assert!(instructions.contains("<skill name=\"code-review\">"));
                assert!(instructions.contains("Review the code carefully"));
                assert!(instructions.contains("Check logic"));
                assert!(instructions.contains("</skill>"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_attach_skill_with_files_discovered() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "data-pipeline".to_string(),
            "Build data pipelines".to_string(),
            "# Instructions\nUse the bundled script.".to_string(),
            vec![
                ("run.py".to_string(), "import pandas as pd".to_string()),
                ("README.md".to_string(), "# Reference docs".to_string()),
            ],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // list_skills discovers it
        let context = ToolContext::with_file_store(session_id, store.clone());
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skills"][0]["name"], "data-pipeline");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_multiple_attach_skills_all_discovered() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Mount 3 different skills from "registry"
        for (i, (name, desc)) in [
            ("pdf-tool", "PDF processing"),
            ("csv-analyzer", "CSV analysis"),
            ("code-review", "Code review"),
        ]
        .iter()
        .enumerate()
        {
            let skill_id =
                uuid::Uuid::parse_str(&format!("550e8400-e29b-41d4-a716-44665544000{}", i))
                    .unwrap();
            let cap = AttachSkillCapability::from_registry(
                skill_id,
                name.to_string(),
                desc.to_string(),
                format!("# {name} Instructions"),
                vec![],
            );
            for mount in cap.mounts() {
                materialize_mount_into_store(&store, session_id, &mount);
            }
        }

        // list_skills discovers all 3
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 3);
                let names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
                assert!(names.contains(&"pdf-tool"));
                assert!(names.contains(&"csv-analyzer"));
                assert!(names.contains(&"code-review"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_attach_skill_prompt_contribution_includes_mounted_skill() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "pdf-tool".to_string(),
            "Extract text from PDFs".to_string(),
            "# Instructions".to_string(),
            vec![],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // SkillsCapability dynamic prompt should include the mounted skill
        let skills_cap = SkillsCapability;
        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
            model: None,
        };
        let result = skills_cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("pdf-tool"));
        assert!(result.contains("Extract text from PDFs"));
        assert!(result.contains("Available skills:"));
    }

    #[tokio::test]
    async fn test_attach_skill_description_with_special_chars_roundtrips() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "tricky-skill".to_string(),
            "Description with: colons, #hashtags, and \"quotes\"".to_string(),
            "# Instructions\nDo the thing.".to_string(),
            vec![],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // list_skills should parse the YAML correctly
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0]["name"], "tricky-skill");
                assert_eq!(
                    skills[0]["description"],
                    "Description with: colons, #hashtags, and \"quotes\""
                );
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // ========================================================================
    // activate_skill env var substitution tests
    // ========================================================================

    #[tokio::test]
    async fn test_activate_skill_substitutes_session_id() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let skill_md =
            "---\nname: test-env\ndescription: Test env vars.\n---\n\nSession: ${SESSION_ID}";
        fs.add_file(session_id, "/.agents/skills/test-env/SKILL.md", skill_md);

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "test-env"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let instructions = val["instructions"].as_str().unwrap();
                let expected_id = session_id.to_string();
                assert!(
                    instructions.contains(&expected_id),
                    "Instructions should contain session ID '{}', got: {}",
                    expected_id,
                    instructions
                );
                assert!(
                    !instructions.contains("${SESSION_ID}"),
                    "Raw placeholder should be replaced"
                );
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_substitutes_skill_dir() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let skill_md =
            "---\nname: test-dir\ndescription: Test skill dir.\n---\n\nDir: ${SKILL_DIR}";
        fs.add_file(session_id, "/.agents/skills/test-dir/SKILL.md", skill_md);

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "test-dir"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let instructions = val["instructions"].as_str().unwrap();
                assert!(
                    instructions.contains("/.agents/skills/test-dir"),
                    "Instructions should contain skill dir path, got: {}",
                    instructions
                );
                assert!(
                    !instructions.contains("${SKILL_DIR}"),
                    "Raw placeholder should be replaced"
                );
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_substitutes_both_env_vars() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let skill_md = "---\nname: test-both\ndescription: Both vars.\n---\n\n${SKILL_DIR}/run.sh --session ${SESSION_ID}";
        fs.add_file(session_id, "/.agents/skills/test-both/SKILL.md", skill_md);

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "test-both"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let instructions = val["instructions"].as_str().unwrap();
                let expected_id = session_id.to_string();
                assert!(instructions.contains("/.agents/skills/test-both/run.sh"));
                assert!(instructions.contains(&format!("--session {}", expected_id)));
                assert!(!instructions.contains("${SESSION_ID}"));
                assert!(!instructions.contains("${SKILL_DIR}"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // Regression test for PR #1449 / EVE-388: SKILL.md written via the VFS
    // at runtime must leave `!`command`` placeholders literal — no shell
    // execution on the worker host.
    #[tokio::test]
    async fn test_activate_skill_does_not_execute_commands_for_writable_skill() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let skill_md = "---\nname: test-no-exec\ndescription: Leaves command placeholders literal.\n---\n\nLiteral: !`echo pwned`";
        // add_file — writable (user-authored) source.
        fs.add_file(
            session_id,
            "/.agents/skills/test-no-exec/SKILL.md",
            skill_md,
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "test-no-exec"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let instructions = val["instructions"].as_str().unwrap();
                assert!(
                    instructions.contains("Literal: !`echo pwned`"),
                    "writable SKILL.md must not execute commands; got: {}",
                    instructions
                );
                assert!(
                    !instructions.contains("pwned\n") && !instructions.contains("\npwned"),
                    "command output must not appear in skill instructions; got: {}",
                    instructions
                );
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // Regression test for the Copilot review of EVE-388: a SKILL.md whose
    // `is_readonly = true` does NOT earn trust. Users can set `is_readonly`
    // via the session-files API and `InitialFile`, so it is not a valid
    // provenance signal. The trust gate must remain off until a
    // platform-controlled signal (e.g. `mount_capability_id`) exists.
    #[tokio::test]
    async fn test_activate_skill_does_not_execute_commands_for_readonly_user_skill() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        let skill_md = "---\nname: test-readonly-no-exec\ndescription: is_readonly alone must not unlock exec.\n---\n\nLiteral: !`echo pwned`";
        // add_readonly_file simulates a user marking a SKILL.md is_readonly=true
        // via the session-files API or InitialFile configuration.
        fs.add_readonly_file(
            session_id,
            "/.agents/skills/test-readonly-no-exec/SKILL.md",
            skill_md,
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(
                serde_json::json!({"name": "test-readonly-no-exec"}),
                &context,
            )
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let instructions = val["instructions"].as_str().unwrap();
                assert!(
                    instructions.contains("Literal: !`echo pwned`"),
                    "is_readonly=true must not bypass the command-substitution gate; got: {}",
                    instructions
                );
                assert!(
                    !instructions.contains("pwned\n") && !instructions.contains("\npwned"),
                    "command output must not appear in skill instructions; got: {}",
                    instructions
                );
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_contribution_excludes_disable_model_invocation_skills() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Normal skill
        store.add_file(
            session_id,
            "/.agents/skills/normal-skill/SKILL.md",
            &valid_skill_md("normal-skill", "A normal skill"),
        );

        // Skill with disable-model-invocation: true
        store.add_file(
            session_id,
            "/.agents/skills/manual-skill/SKILL.md",
            "---\nname: manual-skill\ndescription: Manual only skill\ndisable-model-invocation: true\n---\n\n# Instructions\nManual only.",
        );

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
            model: None,
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(
            result.contains("normal-skill"),
            "Normal skill should appear"
        );
        assert!(
            !result.contains("manual-skill"),
            "Skill with disable-model-invocation should not appear in system prompt"
        );
    }

    #[tokio::test]
    async fn test_contribution_truncates_long_descriptions() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        let long_desc = "a".repeat(200);
        store.add_file(
            session_id,
            "/.agents/skills/long-desc/SKILL.md",
            &valid_skill_md("long-desc", &long_desc),
        );

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
            model: None,
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();

        // The full 200-char description should NOT appear
        assert!(!result.contains(&long_desc));
        // Should contain truncated version with ellipsis
        assert!(result.contains('…'));
    }

    #[test]
    fn localized_name_differs_from_default() {
        let cap = SkillsCapability;
        assert_ne!(cap.localized_name(Some("uk")), cap.name());
    }

    #[tokio::test]
    async fn activation_preserves_literal_and_empty_arguments_through_vfs_pipeline() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(session_id,"/.agents/skills/args/SKILL.md","---\nname: args\ndescription: Literal arguments.\n---\n$ARGUMENTS[0]|$1|$2|${SKILL_DIR}|!`unused-command`");
        let context = ToolContext::with_file_store(session_id, fs);
        let result = ActivateSkillFromVfsTool
            .execute_with_context(
                serde_json::json!({"name":"args","arguments":"'$1' \"\" tail"}),
                &context,
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success: {result:?}")
        };
        assert_eq!(
            value,
            serde_json::json!({"skill":"args","description":"Literal arguments.","instructions":"<skill name=\"args\">\n$1||tail|/.agents/skills/args|!`unused-command`\n</skill>"})
        );
    }
}
