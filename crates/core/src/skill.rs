// Skill domain types and SKILL.md parser
//
// Skills are portable instruction packages following the agentskills.io format.
// A skill consists of a SKILL.md file (YAML frontmatter + markdown body)
// with optional bundled scripts, references, and assets.

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use tracing::warn;

/// Match argument placeholders in the template, never in inserted argument values.
static ARGUMENTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$ARGUMENTS(?:\[([0-9]+)\])?|\$([0-9])").unwrap());

/// Cached regex for ``!`command` `` dynamic command injection syntax.
static COMMAND_INJECTION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!`([^`]+)`").unwrap());

use crate::typed_id::SkillId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Skill source type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum SkillSourceType {
    /// Single SKILL.md file (instructions only)
    Markdown,
    /// ZIP archive with SKILL.md + scripts/references/assets
    Archive,
}

impl std::fmt::Display for SkillSourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillSourceType::Markdown => write!(f, "markdown"),
            SkillSourceType::Archive => write!(f, "archive"),
        }
    }
}

impl From<&str> for SkillSourceType {
    fn from(s: &str) -> Self {
        match s {
            "archive" => SkillSourceType::Archive,
            _ => SkillSourceType::Markdown,
        }
    }
}

/// Skill lifecycle status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum SkillStatus {
    Active,
    Disabled,
    Archived,
    Deleted,
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillStatus::Active => write!(f, "active"),
            SkillStatus::Disabled => write!(f, "disabled"),
            SkillStatus::Archived => write!(f, "archived"),
            SkillStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for SkillStatus {
    fn from(s: &str) -> Self {
        match s {
            "disabled" => SkillStatus::Disabled,
            "archived" => SkillStatus::Archived,
            "deleted" => SkillStatus::Deleted,
            _ => SkillStatus::Active,
        }
    }
}

/// Skill entity (API response type)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Skill {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "skill_01933b5a00007000800000000000001"))]
    pub id: SkillId,
    /// Stable kebab-case slug used to invoke the skill (e.g. `/pdf-processing` in chat). Safe to render in user-facing messages.
    #[cfg_attr(feature = "openapi", schema(example = "pdf-processing"))]
    pub name: String,
    /// Short, agent- and user-readable summary of what the skill does and when to use it.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Extract text and tables from PDF files.")
    )]
    pub description: String,
    /// License string as declared by the skill author (e.g. `MIT`, `Apache-2.0`). Informational; not enforced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Compatibility marker describing host-runtime requirements declared by the skill (e.g. min platform version). Informational.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    /// Free-form metadata declared by the skill author.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Comma-separated list of tool patterns this skill may invoke. `None` means inherit from the harness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
    /// How the skill content is sourced (filesystem, URL, embedded). Determines reload semantics.
    pub source_type: SkillSourceType,
    /// Current lifecycle status (`active`, `archived`, `deleted`).
    pub status: SkillStatus,
    /// Semver string declared by the skill author. Free-form; sorted lexicographically when comparing.
    pub version: String,
    /// Whether this skill appears as a `/`-prefixed slash command for end users in chat UIs.
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    /// When `true`, the LLM is prevented from auto-invoking this skill; only the user can trigger it explicitly.
    #[serde(default)]
    pub disable_model_invocation: bool,
    /// Timestamp when this skill was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this skill was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
    /// Timestamp when this skill was archived, if any (RFC 3339). Archived skills are hidden from default list views.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when this skill was hard-deleted, if any (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Skill execution context mode.
///
/// Determines whether the skill runs inline in the current session
/// or in an isolated subagent context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SkillContext {
    /// Run inline in the current session (default)
    #[default]
    Inline,
    /// Run in an isolated subagent session
    Fork,
}

impl std::fmt::Display for SkillContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillContext::Inline => write!(f, "inline"),
            SkillContext::Fork => write!(f, "fork"),
        }
    }
}

/// Parsed SKILL.md content
#[derive(Debug, Clone)]
pub struct ParsedSkillMd {
    /// Skill name from frontmatter
    pub name: String,
    /// Description from frontmatter
    pub description: String,
    /// License from frontmatter
    pub license: Option<String>,
    /// Compatibility from frontmatter
    pub compatibility: Option<String>,
    /// Arbitrary metadata from frontmatter
    pub metadata: HashMap<String, serde_json::Value>,
    /// Allowed tools from frontmatter
    pub allowed_tools: Option<String>,
    /// Version from metadata or default
    pub version: String,
    /// Markdown body (after frontmatter)
    pub instructions: String,
    /// Whether this skill appears as a /slash command for users (default: true)
    pub user_invocable: bool,
    /// Whether the model is prevented from auto-invoking this skill (default: false)
    pub disable_model_invocation: bool,
    /// Hint string for autocomplete (e.g., `"<issue-number>"`)
    pub argument_hint: Option<String>,
    /// Execution context: inline (default) or fork (subagent)
    pub context: SkillContext,
    /// Subagent type when context is fork (e.g., "Explore", "Plan"). Default: "general-purpose"
    pub agent: Option<String>,
    /// LLM model override for this skill (e.g., "claude-haiku-4-5-20251001")
    pub model: Option<String>,
}

/// YAML frontmatter structure
#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    license: Option<String>,
    compatibility: Option<String>,
    #[serde(default)]
    metadata: HashMap<String, serde_json::Value>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<String>,
    /// Whether this skill appears as a /slash command (default: true)
    #[serde(rename = "user-invocable", default = "default_true")]
    user_invocable: bool,
    /// Whether the model is prevented from auto-invoking this skill (default: false)
    #[serde(rename = "disable-model-invocation", default)]
    disable_model_invocation: bool,
    /// Hint string shown in autocomplete for expected arguments
    #[serde(rename = "argument-hint")]
    argument_hint: Option<String>,
    /// Execution context: "fork" runs in isolated subagent, absent/other = inline
    context: Option<String>,
    /// Subagent type when context is fork (e.g., "Explore", "Plan")
    agent: Option<String>,
    /// LLM model override for this skill
    model: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Skill content response (for /content endpoint)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SkillContent {
    pub skill_md: String,
    pub files: Vec<SkillFileEntry>,
}

/// A file entry in a skill archive
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SkillFileEntry {
    pub path: String,
    pub content: String,
}

/// Number of agents and harnesses that reference a skill via its
/// `skill:{uuid}` capability id. The `/v1/skills/usage` endpoint returns this
/// keyed by public `SkillId`; skills with no references are omitted from the
/// map and the UI defaults missing entries to zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SkillUsage {
    pub agents: u64,
    pub harnesses: u64,
}

/// Validation result for SKILL.md
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SkillValidationResult {
    /// `true` when the candidate SKILL.md parsed and passes all hard checks; `false` if any error was found.
    pub valid: bool,
    /// Parsed skill slug from the front matter. `None` when the input could not be parsed enough to extract a name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parsed skill description. `None` when not present in the input or unparseable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Hard validation errors. Non-empty if and only if `valid` is `false`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Non-fatal warnings (style, deprecated patterns, optional fields missing). Emitted alongside a `valid` result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

// ============================================================================
// SKILL.md Parser
// ============================================================================

/// Parse a SKILL.md string into structured data.
///
/// Uses a two-pass strategy: strict `serde_yaml` first, then a lenient
/// fallback that auto-fixes common issues (unquoted colons, special chars)
/// before rejecting the skill entirely. Logs a warning when fallback is used.
pub fn parse_skill_md(content: &str) -> Result<ParsedSkillMd, Vec<String>> {
    let (frontmatter_str, body) = extract_frontmatter(content)?;
    let fm: SkillFrontmatter = match serde_yaml::from_str(&frontmatter_str) {
        Ok(fm) => fm,
        Err(strict_err) => match try_lenient_yaml_parse(&frontmatter_str) {
            Ok(fm) => {
                warn!(
                    strict_error = %strict_err,
                    "SKILL.md YAML frontmatter required lenient parsing; skill authors should fix their YAML."
                );
                fm
            }
            Err(_) => {
                return Err(vec![format!("invalid YAML frontmatter: {strict_err}")]);
            }
        },
    };

    let mut errors = Vec::new();

    let name = match &fm.name {
        Some(n) => {
            if let Err(name_errors) = validate_skill_name(n) {
                errors.extend(name_errors);
            }
            n.clone()
        }
        None => {
            errors.push("name: required field missing".to_string());
            String::new()
        }
    };

    let description = match &fm.description {
        Some(d) if d.trim().is_empty() => {
            errors.push("description: must not be empty".to_string());
            String::new()
        }
        Some(d) if d.len() > 1024 => {
            errors.push("description: exceeds 1024 character limit".to_string());
            d.clone()
        }
        Some(d) => d.clone(),
        None => {
            errors.push("description: required field missing".to_string());
            String::new()
        }
    };

    if let Some(ref license) = fm.license
        && license.len() > 500
    {
        errors.push("license: exceeds 500 character limit".to_string());
    }

    if let Some(ref compat) = fm.compatibility
        && compat.len() > 500
    {
        errors.push("compatibility: exceeds 500 character limit".to_string());
    }

    if let Some(ref hint) = fm.argument_hint
        && hint.len() > 128
    {
        errors.push("argument-hint: exceeds 128 character limit".to_string());
    }

    // Parse context field
    let context = match fm.context.as_deref() {
        Some("fork") => SkillContext::Fork,
        Some("inline") | None => SkillContext::Inline,
        Some(other) => {
            errors.push(format!(
                "context: invalid value \"{other}\", must be \"fork\" or \"inline\""
            ));
            SkillContext::Inline
        }
    };

    // Validate agent field only meaningful with context: fork
    if fm.agent.is_some() && context != SkillContext::Fork {
        errors.push("agent: field is only meaningful when context is \"fork\"".to_string());
    }

    if body.len() > 100 * 1024 {
        errors.push("instructions: exceeds 100 KB limit".to_string());
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let version = fm
        .metadata
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("1.0")
        .to_string();

    Ok(ParsedSkillMd {
        name,
        description,
        license: fm.license,
        compatibility: fm.compatibility,
        metadata: fm.metadata,
        allowed_tools: fm.allowed_tools,
        version,
        instructions: body,
        user_invocable: fm.user_invocable,
        disable_model_invocation: fm.disable_model_invocation,
        argument_hint: fm.argument_hint,
        context,
        agent: fm.agent,
        model: fm.model,
    })
}

/// Validate a SKILL.md and return a SkillValidationResult
pub fn validate_skill_md(content: &str) -> SkillValidationResult {
    match parse_skill_md(content) {
        Ok(parsed) => {
            let mut warnings = Vec::new();
            let line_count = parsed.instructions.lines().count();
            if line_count > 500 {
                warnings.push(format!(
                    "Instructions exceed 500 lines ({line_count} lines). Consider splitting into references."
                ));
            }
            if !parsed.user_invocable && parsed.disable_model_invocation {
                warnings.push(
                    "Skill is unreachable: user-invocable is false and disable-model-invocation is true. \
                     Neither users nor the model can invoke this skill."
                        .to_string(),
                );
            }
            if parsed.context == SkillContext::Fork && parsed.agent.is_none() {
                warnings.push(
                    "context: fork without agent field — will use default \"general-purpose\" agent."
                        .to_string(),
                );
            }
            if parsed.model.is_some() && parsed.context != SkillContext::Fork {
                warnings.push(
                    "model: field is only supported with context: fork. \
                     Inline skills ignore the model override."
                        .to_string(),
                );
            }
            SkillValidationResult {
                valid: true,
                name: Some(parsed.name),
                description: Some(parsed.description),
                errors: vec![],
                warnings,
            }
        }
        Err(errors) => SkillValidationResult {
            valid: false,
            name: None,
            description: None,
            errors,
            warnings: vec![],
        },
    }
}

/// Validate a skill name per agentskills.io spec
pub fn validate_skill_name(name: &str) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if name.is_empty() || name.len() > 64 {
        errors.push("name: must be 1-64 characters".to_string());
    }

    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        errors.push("name: must contain only lowercase letters, numbers, and hyphens".to_string());
    }

    if name.starts_with('-') || name.ends_with('-') {
        errors.push("name: must not start or end with hyphen".to_string());
    }

    if name.contains("--") {
        errors.push("name: must not contain consecutive hyphens".to_string());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Extract YAML frontmatter and body from a SKILL.md string.
/// Frontmatter is delimited by `---` lines.
fn extract_frontmatter(content: &str) -> Result<(String, String), Vec<String>> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(vec![
            "SKILL.md must start with YAML frontmatter (--- delimiter)".to_string(),
        ]);
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let closing = after_first
        .find("\n---")
        .ok_or_else(|| vec!["SKILL.md frontmatter missing closing --- delimiter".to_string()])?;

    let frontmatter = &after_first[..closing];
    let body_start = closing + 4; // skip "\n---"
    let body = if body_start < after_first.len() {
        after_first[body_start..]
            .trim_start_matches('\n')
            .to_string()
    } else {
        String::new()
    };

    Ok((frontmatter.to_string(), body))
}

/// Attempt lenient YAML parsing by auto-fixing common issues:
/// - Unquoted values containing colons (e.g., `description: Use this: it works`)
/// - Unquoted values with special YAML characters (`{`, `}`, `[`, `]`, `#`)
/// - Strip invalid control characters (except tab; newlines consumed by line iteration)
fn try_lenient_yaml_parse(frontmatter: &str) -> Result<SkillFrontmatter, serde_yaml::Error> {
    let fixed = fix_yaml_values(frontmatter);
    serde_yaml::from_str(&fixed)
}

/// Auto-quote YAML values that contain problematic characters.
///
/// For each line that looks like `key: value`, if the value is not already
/// quoted and contains characters that break strict YAML parsing (`:`, `{`,
/// `}`, `[`, `]`, `#`), wrap it in double quotes (escaping inner quotes).
/// Also strips control characters (except `\t`; `\n` is consumed by line iteration).
fn fix_yaml_values(frontmatter: &str) -> String {
    let problematic_chars: &[char] = &[':', '{', '}', '[', ']', '#'];

    frontmatter
        .lines()
        .map(|line| {
            // Strip invalid control characters (keep \t)
            let line: String = line
                .chars()
                .filter(|c| !c.is_control() || *c == '\t')
                .collect();

            // Match `key: value` pattern (top-level only, no leading whitespace for nested)
            if let Some(colon_pos) = line.find(": ") {
                let key = &line[..colon_pos];
                let value = line[colon_pos + 2..].trim();

                // Skip if already quoted, empty, or a nested/list structure
                if value.is_empty()
                    || value.starts_with('"')
                    || value.starts_with('\'')
                    || value.starts_with('|')
                    || value.starts_with('>')
                    || key.starts_with(' ')
                    || key.starts_with('\t')
                {
                    return line;
                }

                // If value contains problematic chars, quote it.
                // Skip values that look like YAML flow collections (start with { or [).
                if value.contains(problematic_chars)
                    && !value.starts_with('{')
                    && !value.starts_with('[')
                {
                    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                    return format!("{key}: \"{escaped}\"");
                }
            }

            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================================
// Skill Argument Substitution
// ============================================================================

/// Split arguments respecting quoted strings.
///
/// Splits on whitespace, treating `"hello world"` or `'hello world'` as single tokens.
/// Quotes are stripped from the result.
fn split_skill_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut token_started = false;
    for c in raw.chars() {
        match (c, in_quote) {
            ('"' | '\'', None) => {
                in_quote = Some(c);
                token_started = true;
            }
            (q, Some(open)) if q == open => in_quote = None,
            (c, Some(_)) => current.push(c),
            (c, None) if c.is_whitespace() => {
                if token_started {
                    args.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            (c, None) => {
                current.push(c);
                token_started = true;
            }
        }
    }
    if token_started {
        args.push(current);
    }
    args
}

/// Expand positional argument placeholders in skill content.
///
/// Supported template variables (inserted values are not expanded again):
/// 1. `$ARGUMENTS[N]` → Nth positional argument (0-based)
/// 2. `$ARGUMENTS` → full argument string
/// 3. `$N` (single digit 0-9) → shorthand for `$ARGUMENTS[N]`
///
/// If no placeholders are found and arguments are non-empty, appends `ARGUMENTS: <value>`.
/// Out-of-bounds indices resolve to empty string.
pub fn expand_skill_arguments(content: &str, raw_args: &str) -> String {
    if raw_args.is_empty() {
        return content.to_string();
    }

    let args = split_skill_args(raw_args);
    let mut had_placeholder = false;
    // Only scan the original template: values such as "$1" are data, not another template.
    let mut result = ARGUMENTS_RE
        .replace_all(content, |caps: &regex::Captures| {
            let matched = caps.get(0).unwrap();
            if let Some(digit) = caps.get(2) {
                if content[matched.end()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return matched.as_str().to_string();
                }
                had_placeholder = true;
                let index = (digit.as_str().as_bytes()[0] - b'0') as usize;
                args.get(index).cloned().unwrap_or_default()
            } else if let Some(index) = caps.get(1) {
                had_placeholder = true;
                let index = index.as_str().parse::<usize>().unwrap_or(usize::MAX);
                args.get(index).cloned().unwrap_or_default()
            } else {
                had_placeholder = true;
                raw_args.to_string()
            }
        })
        .to_string();
    if !had_placeholder {
        result.push_str(&format!("\n\nARGUMENTS: {}", raw_args));
    }

    result
}

// ============================================================================
// Environment Variable Substitution
// ============================================================================

/// Substitute activation-time placeholders in skill content.
///
/// Replaces:
/// - `${SESSION_ID}` → current session's prefixed ID (e.g. `session_01abc...`)
/// - `${SKILL_DIR}` → absolute path to the skill's directory
///
/// Called after `$ARGUMENTS`/`$N` substitution, before `!command` preprocessing.
pub fn substitute_activation_vars(content: &str, session_id: &str, skill_dir: &str) -> String {
    content
        .replace("${SESSION_ID}", session_id)
        .replace("${SKILL_DIR}", skill_dir)
}

// ============================================================================
// Dynamic Context Injection: !`command` preprocessing
// ============================================================================
//
// TRUSTED-SOURCE GATE: `preprocess_command_injections` executes shell commands
// embedded in SKILL.md, which is RCE if the SKILL.md came from an attacker.
// Callers MUST only invoke this function when the SKILL.md originates from a
// non-user-spoofable source (e.g. a capability/registry-owned virtual mount).
//
// `SessionFile::is_readonly` is NOT a valid trust signal: it is user-settable
// via the session-files HTTP API and via `InitialFile` configuration. A
// future platform-controlled provenance field (for example, a
// `mount_capability_id` populated only by mount application code) is needed
// before this function can be used for any runtime source. Until then,
// `ActivateSkillFromVfsTool::execute_with_context` leaves command placeholders
// literal; this neutral helper is preserved for its unit tests and a future
// sandbox-backed integration.
//
// When command substitution is re-enabled, its executor MUST be a
// session-sandbox-backed implementation so
// commands run against the bashkit shell (managed session sandbox) and
// the session virtual filesystem rather than the worker. Flipping the trust
// gate without that replacement would still be RCE against the worker host.
//
// See `knowledge/project/skills-registry.md` ("Activation Substitution Pipeline") and
// `knowledge/security/threat-model.md` entry TM-TOOL-020 for the rationale.

/// Result of executing a shell command during skill preprocessing.
pub struct CommandResult {
    pub stdout: String,
    pub exit_code: i32,
}

/// Trait for executing shell commands during skill preprocessing.
///
/// Commands in `!`...`` syntax would be executed before the skill content
/// is sent to the model, replacing each placeholder with command output.
///
/// This path is intentionally not reached at runtime today; see the
/// trust-gate note at the top of this module.
#[async_trait::async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute_command(&self, command: &str) -> CommandResult;
}

/// Maximum number of `!`command`` placeholders expanded per activation.
///
/// Excess placeholders are replaced with a sentinel error; they are not
/// executed. Bounds the shell-process fan-out even for a trusted SKILL.md.
pub const MAX_COMMAND_PLACEHOLDERS_PER_SKILL: usize = 32;

/// Maximum number of `!`command`` placeholders executed concurrently within
/// a single activation. Keeps worker process pressure bounded under load.
const COMMAND_EXECUTION_CONCURRENCY: usize = 4;

/// Preprocess `!`command`` placeholders in skill content.
///
/// Each `!`command`` is executed via the provided executor and replaced with
/// its stdout. Execution is bounded: at most
/// [`MAX_COMMAND_PLACEHOLDERS_PER_SKILL`] placeholders are expanded per call
/// (extras are replaced with `[Too many command placeholders: limit is N]`
/// sentinels), and at most `COMMAND_EXECUTION_CONCURRENCY` commands run
/// concurrently.
///
/// Substitution pipeline order (caller is responsible for prior steps):
/// 1. `$ARGUMENTS` / `$N` substitution (sync)
/// 2. `${SESSION_ID}` / `${SKILL_DIR}` env substitution (sync)
/// 3. `!`command`` preprocessing (async) — this function
///
/// SECURITY: This function spawns shell processes on the worker host. It MUST
/// only be called for skill content that came from a trusted source (see the
/// trust-gate note at the top of this module). Untrusted content must bypass
/// this step and be used verbatim.
pub async fn preprocess_command_injections(
    content: &str,
    executor: &dyn CommandExecutor,
) -> String {
    use futures::stream::StreamExt;

    let all_matches: Vec<(String, std::ops::Range<usize>)> = COMMAND_INJECTION_RE
        .captures_iter(content)
        .map(|cap| {
            let full = cap.get(0).unwrap();
            let cmd = cap[1].to_string();
            (cmd, full.start()..full.end())
        })
        .collect();

    if all_matches.is_empty() {
        return content.to_string();
    }

    // Partition at the cap: the first N are executed, the rest get a sentinel
    // replacement so the content still carries a visible marker but no extra
    // shell processes are spawned.
    let exec_count = all_matches.len().min(MAX_COMMAND_PLACEHOLDERS_PER_SKILL);

    // `buffered` preserves input order, so results line up with
    // `all_matches[..exec_count]` positionally. We collect owned command
    // strings so the stream items are `'static`, side-stepping a borrow-
    // across-await lifetime that the compiler otherwise rejects.
    let cmds_to_run: Vec<String> = all_matches[..exec_count]
        .iter()
        .map(|(cmd, _)| cmd.clone())
        .collect();
    let results: Vec<CommandResult> = futures::stream::iter(cmds_to_run)
        .map(|cmd| async move { executor.execute_command(&cmd).await })
        .buffered(COMMAND_EXECUTION_CONCURRENCY)
        .collect()
        .await;

    let mut result = content.to_string();
    // Walk all matches in reverse so byte ranges remain valid as we splice.
    // For positions below exec_count, use the command result; for positions
    // above (only possible when exceeded_cap), substitute the cap sentinel.
    for (idx, (cmd, range)) in all_matches.iter().enumerate().rev() {
        let replacement = if idx < exec_count {
            let cmd_result = &results[idx];
            if cmd_result.exit_code != 0 && cmd_result.stdout.starts_with('[') {
                cmd_result.stdout.clone()
            } else if cmd_result.exit_code != 0 {
                format!(
                    "[Command failed: {} (exit code {})]",
                    cmd, cmd_result.exit_code
                )
            } else if cmd_result.stdout.is_empty() {
                "[No output]".to_string()
            } else {
                cmd_result.stdout.trim_end().to_string()
            }
        } else {
            format!(
                "[Too many command placeholders: limit is {}]",
                MAX_COMMAND_PLACEHOLDERS_PER_SKILL
            )
        };
        result.replace_range(range.clone(), &replacement);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_skill_md() {
        let content = r#"---
name: pdf-processing
description: Extract text from PDF files.
---

# PDF Processing

Use pdfplumber to extract text.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.name, "pdf-processing");
        assert_eq!(parsed.description, "Extract text from PDF files.");
        assert_eq!(
            parsed.instructions,
            "# PDF Processing\n\nUse pdfplumber to extract text.\n"
        );
        assert!(parsed.user_invocable);
        assert!(!parsed.disable_model_invocation);
        assert_eq!(parsed.argument_hint, None);
        assert_eq!(parsed.context, SkillContext::Inline);
        assert_eq!(parsed.agent, None);
        assert_eq!(parsed.model, None);
        assert_eq!(parsed.version, "1.0");
    }

    #[test]
    fn test_parse_with_optional_fields() {
        let content = r#"---
name: data-analysis
description: Analyze datasets.
license: MIT
compatibility: Python 3.10+
metadata:
  version: "2.0"
  author: test
allowed-tools: bash python
---

Instructions here.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.name, "data-analysis");
        assert_eq!(parsed.license.as_deref(), Some("MIT"));
        assert_eq!(parsed.compatibility.as_deref(), Some("Python 3.10+"));
        assert_eq!(parsed.version, "2.0");
        assert_eq!(parsed.allowed_tools.as_deref(), Some("bash python"));
    }

    #[test]
    fn test_parse_missing_name() {
        let content = r#"---
description: No name here.
---

Body.
"#;
        let err = parse_skill_md(content).unwrap_err();
        assert!(err.iter().any(|e| e.contains("name: required")));
    }

    #[test]
    fn test_parse_missing_description() {
        let content = r#"---
name: test-skill
---

Body.
"#;
        let err = parse_skill_md(content).unwrap_err();
        assert!(err.iter().any(|e| e.contains("description: required")));
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "# Just markdown, no frontmatter";
        let err = parse_skill_md(content).unwrap_err();
        assert!(err.iter().any(|e| e.contains("frontmatter")));
    }

    #[test]
    fn test_validate_name_valid() {
        assert!(validate_skill_name("pdf-processing").is_ok());
        assert!(validate_skill_name("a").is_ok());
        assert!(validate_skill_name("my-skill-123").is_ok());
    }

    #[test]
    fn test_validate_name_invalid() {
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("-leading").is_err());
        assert!(validate_skill_name("trailing-").is_err());
        assert!(validate_skill_name("double--hyphen").is_err());
        assert!(validate_skill_name("UPPERCASE").is_err());
        assert!(validate_skill_name("has spaces").is_err());
        assert!(validate_skill_name("has_underscores").is_err());
    }

    #[test]
    fn test_validate_skill_md() {
        let content = r#"---
name: test-skill
description: A test skill.
---

Instructions.
"#;
        let result = validate_skill_md(content);
        assert!(result.valid);
        assert_eq!(result.name.as_deref(), Some("test-skill"));
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_skill_md_invalid() {
        let content = r#"---
name: INVALID
---

Body.
"#;
        let result = validate_skill_md(content);
        assert!(!result.valid);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_parse_argument_hint() {
        let content = r#"---
name: fix-issue
description: Fix a GitHub issue.
argument-hint: "<issue-number>"
---

Fix issue $ARGUMENTS.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.argument_hint.as_deref(), Some("<issue-number>"));
    }

    // ========================================================================
    // context and agent frontmatter tests
    // ========================================================================

    #[test]
    fn test_parse_context_fork() {
        let content = r#"---
name: deep-research
description: Research a topic thoroughly.
context: fork
---

Research $ARGUMENTS.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.context, SkillContext::Fork);
        assert!(parsed.agent.is_none());
    }

    #[test]
    fn test_parse_context_fork_with_agent() {
        let content = r#"---
name: explore-code
description: Explore codebase.
context: fork
agent: Explore
---

Explore $ARGUMENTS.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.context, SkillContext::Fork);
        assert_eq!(parsed.agent.as_deref(), Some("Explore"));
    }

    #[test]
    fn test_parse_context_inline_explicit() {
        let content = r#"---
name: my-skill
description: A skill.
context: inline
---

Body.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.context, SkillContext::Inline);
    }

    #[test]
    fn test_parse_context_invalid_value() {
        let content = r#"---
name: my-skill
description: A skill.
context: parallel
---

Body.
"#;
        let err = parse_skill_md(content).unwrap_err();
        assert!(err.iter().any(|e| e.contains("context: invalid value")));
    }

    #[test]
    fn test_parse_agent_without_fork_is_error() {
        let content = r#"---
name: my-skill
description: A skill.
agent: Explore
---

Body.
"#;
        let err = parse_skill_md(content).unwrap_err();
        assert!(
            err.iter()
                .any(|e| e.contains("agent: field is only meaningful"))
        );
    }

    #[test]
    fn test_validate_warns_fork_without_agent() {
        let content = r#"---
name: my-skill
description: A skill.
context: fork
---

Body.
"#;
        let result = validate_skill_md(content);
        assert!(result.valid);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("general-purpose"))
        );
    }

    // -- model frontmatter tests --

    #[test]
    fn test_parse_model_with_fork() {
        let content = r#"---
name: quick-lint
description: Fast lint check.
context: fork
model: claude-haiku-4-5-20251001
---

Lint instructions.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("claude-haiku-4-5-20251001"));
        assert_eq!(parsed.context, SkillContext::Fork);
    }

    #[test]
    fn test_parse_model_without_fork() {
        let content = r#"---
name: my-skill
description: A skill.
model: gpt-5.2
---

Body.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("gpt-5.2"));
        assert_eq!(parsed.context, SkillContext::Inline);
    }

    #[test]
    fn test_validate_warns_model_without_fork() {
        let content = r#"---
name: my-skill
description: A skill.
model: gpt-5.2
---

Body.
"#;
        let result = validate_skill_md(content);
        assert!(result.valid);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("model:") && w.contains("context: fork"))
        );
    }

    #[test]
    fn test_validate_no_warning_model_with_fork() {
        let content = r#"---
name: my-skill
description: A skill.
context: fork
agent: Explore
model: claude-haiku-4-5-20251001
---

Body.
"#;
        let result = validate_skill_md(content);
        assert!(result.valid);
        assert!(
            !result
                .warnings
                .iter()
                .any(|w| w.contains("model:") && w.contains("context: fork"))
        );
    }

    // ========================================================================
    // expand_skill_arguments tests
    // ========================================================================

    #[test]
    fn test_expand_full_arguments() {
        let content = "Process $ARGUMENTS now.";
        let result = expand_skill_arguments(content, "SearchBar React");
        assert_eq!(result, "Process SearchBar React now.");
    }

    #[test]
    fn test_expand_indexed_arguments() {
        let content = "Migrate $ARGUMENTS[0] from $ARGUMENTS[1] to $ARGUMENTS[2].";
        let result = expand_skill_arguments(content, "SearchBar React Vue");
        assert_eq!(result, "Migrate SearchBar from React to Vue.");
    }

    #[test]
    fn test_expand_shorthand_arguments() {
        let content = "Component: $0, from: $1, to: $2.";
        let result = expand_skill_arguments(content, "SearchBar React Vue");
        assert_eq!(result, "Component: SearchBar, from: React, to: Vue.");
    }

    #[test]
    fn test_expand_quoted_arguments() {
        let content = "File: $0, message: $1.";
        let result = expand_skill_arguments(content, "app.js \"hello world\"");
        assert_eq!(result, "File: app.js, message: hello world.");
    }

    #[test]
    fn test_expand_out_of_bounds() {
        let content = "A: $0, B: $1, C: $5.";
        let result = expand_skill_arguments(content, "only-one");
        assert_eq!(result, "A: only-one, B: , C: .");
    }

    #[test]
    fn test_expand_no_placeholders_appends() {
        let content = "Do the thing.";
        let result = expand_skill_arguments(content, "some args");
        assert_eq!(result, "Do the thing.\n\nARGUMENTS: some args");
    }

    #[test]
    fn test_expand_empty_args() {
        let content = "Content with $ARGUMENTS placeholder.";
        let result = expand_skill_arguments(content, "");
        assert_eq!(result, "Content with $ARGUMENTS placeholder.");
    }

    #[test]
    fn test_expand_shorthand_no_word_collision() {
        // $NAME should NOT be replaced (not $0-$9 pattern)
        let content = "Variable $NAME and $0.";
        let result = expand_skill_arguments(content, "first");
        assert_eq!(result, "Variable $NAME and first.");
    }

    #[test]
    fn test_expand_dollar_followed_by_multi_digit() {
        // $10 should NOT match $1 + "0" — only single-digit shorthand
        let content = "Value: $10 and $1.";
        let result = expand_skill_arguments(content, "a b");
        // $10 is not a valid shorthand (digit followed by digit), $1 = "b"
        assert_eq!(result, "Value: $10 and b.");
    }

    #[test]
    fn test_split_skill_args_basic() {
        let args = split_skill_args("a b c");
        assert_eq!(args, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_split_skill_args_quoted() {
        let args = split_skill_args("\"hello world\" foo 'bar baz'");
        assert_eq!(args, vec!["hello world", "foo", "bar baz"]);
    }

    #[test]
    fn test_split_skill_args_empty() {
        let args = split_skill_args("");
        assert!(args.is_empty());
    }

    #[test]
    fn test_split_skill_args_extra_whitespace() {
        let args = split_skill_args("  a   b  ");
        assert_eq!(args, vec!["a", "b"]);
    }

    // ========================================================================
    // substitute_activation_vars tests
    // ========================================================================

    // ========================================================================
    // preprocess_command_injections tests
    // ========================================================================

    /// Mock executor for testing command injection preprocessing.
    struct MockExecutor {
        responses: std::collections::HashMap<String, CommandResult>,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self {
                responses: std::collections::HashMap::new(),
            }
        }

        fn add_response(&mut self, cmd: &str, stdout: &str, exit_code: i32) {
            self.responses.insert(
                cmd.to_string(),
                CommandResult {
                    stdout: stdout.to_string(),
                    exit_code,
                },
            );
        }
    }

    #[async_trait::async_trait]
    impl CommandExecutor for MockExecutor {
        async fn execute_command(&self, command: &str) -> CommandResult {
            self.responses
                .get(command)
                .map(|r| CommandResult {
                    stdout: r.stdout.clone(),
                    exit_code: r.exit_code,
                })
                .unwrap_or(CommandResult {
                    stdout: String::new(),
                    exit_code: 127,
                })
        }
    }

    #[tokio::test]
    async fn test_preprocess_single_command() {
        let mut exec = MockExecutor::new();
        exec.add_response("echo hello", "hello\n", 0);

        let content = "Output: !`echo hello`";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(result, "Output: hello");
    }

    #[tokio::test]
    async fn test_preprocess_multiple_commands() {
        let mut exec = MockExecutor::new();
        exec.add_response("git status", "clean\n", 0);
        exec.add_response("date", "2026-03-19\n", 0);

        let content = "Status: !`git status`\nDate: !`date`";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(result, "Status: clean\nDate: 2026-03-19");
    }

    #[tokio::test]
    async fn test_preprocess_command_failure() {
        let mut exec = MockExecutor::new();
        exec.add_response("bad-cmd", "error output\n", 1);

        let content = "Result: !`bad-cmd`";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(result, "Result: [Command failed: bad-cmd (exit code 1)]");
    }

    #[tokio::test]
    async fn test_preprocess_empty_output() {
        let mut exec = MockExecutor::new();
        exec.add_response("true", "", 0);

        let content = "Result: !`true`";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(result, "Result: [No output]");
    }

    #[tokio::test]
    async fn test_preprocess_no_commands() {
        let exec = MockExecutor::new();

        let content = "No commands here. Just `code` and text.";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(result, content);
    }

    #[tokio::test]
    async fn test_preprocess_preserves_regular_backticks() {
        let mut exec = MockExecutor::new();
        exec.add_response("echo hi", "hi\n", 0);

        let content = "Use `code` and !`echo hi` here.";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(result, "Use `code` and hi here.");
    }

    #[tokio::test]
    async fn test_preprocess_command_not_found() {
        let exec = MockExecutor::new(); // No responses registered

        let content = "Result: !`unknown-cmd`";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(
            result,
            "Result: [Command failed: unknown-cmd (exit code 127)]"
        );
    }

    // -- lenient YAML fallback tests --

    #[test]
    fn test_lenient_parse_unquoted_colon_in_description() {
        let content = r#"---
name: my-skill
description: Use this skill: it handles edge cases
---

Instructions.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.name, "my-skill");
        assert_eq!(parsed.description, "Use this skill: it handles edge cases");
    }

    #[test]
    fn test_parse_hash_inside_plain_value() {
        let content = "---\nname: my-skill\ndescription: Process C# files\n---\n\nBody.\n";
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.description, "Process C# files");
    }

    #[test]
    fn test_parse_embedded_brackets_in_plain_value() {
        let content =
            "---\nname: my-skill\ndescription: Parse [markdown] and {templates}\n---\n\nBody.\n";
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.description, "Parse [markdown] and {templates}");
    }

    #[test]
    fn test_parse_already_quoted_value_unchanged() {
        let content = "---\nname: my-skill\ndescription: \"Already quoted: value\"\n---\n\nBody.\n";
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.description, "Already quoted: value");
    }

    #[test]
    fn test_fix_yaml_values_preserves_clean_yaml() {
        let input = "name: my-skill\ndescription: A simple skill";
        assert_eq!(fix_yaml_values(input), input);
    }

    #[test]
    fn test_fix_yaml_values_quotes_colons() {
        let input = "name: my-skill\ndescription: Use this: it works";
        let fixed = fix_yaml_values(input);
        assert!(fixed.contains("description: \"Use this: it works\""));
    }

    #[test]
    fn test_fix_yaml_values_escapes_inner_quotes() {
        let input = "name: my-skill\ndescription: Say \"hello\": world";
        let fixed = fix_yaml_values(input);
        assert!(fixed.contains(r#"description: "Say \"hello\": world""#));
    }

    #[test]
    fn test_fix_yaml_values_skips_nested_keys() {
        let input = "metadata:\n  version: 1.0\n  key: value: nested";
        let fixed = fix_yaml_values(input);
        // Nested keys (indented) should not be modified
        assert_eq!(fixed, input);
    }

    #[test]
    fn test_fix_yaml_values_preserves_flow_collections() {
        let input = "name: my-skill\nmetadata: { version: \"1.0\" }\ntags: [a, b]";
        let fixed = fix_yaml_values(input);
        assert_eq!(fixed, input);
    }

    #[test]
    fn argument_values_are_inserted_literally_without_reexpansion() {
        assert_eq!(
            expand_skill_arguments("indexed: $ARGUMENTS[0]", "'$1' second"),
            "indexed: $1"
        );
        assert_eq!(
            expand_skill_arguments("full: $ARGUMENTS", "$1 second"),
            "full: $1 second"
        );
        assert_eq!(
            expand_skill_arguments("$ARGUMENTS[0] / $1", "'$ARGUMENTS' second"),
            "$ARGUMENTS / second"
        );
    }

    #[test]
    fn quoted_empty_arguments_preserve_positional_identity() {
        assert_eq!(
            split_skill_args("\"\" next '' last"),
            vec!["", "next", "", "last"]
        );
        assert_eq!(
            expand_skill_arguments("$0|$1|$2|$3", "\"\" next '' last"),
            "|next||last"
        );
    }

    #[test]
    fn invocation_flags_preserve_all_combinations_and_warn_only_when_unreachable() {
        for (user, disabled) in [(false, false), (false, true), (true, false), (true, true)] {
            let content = format!(
                "---\nname: sample\ndescription: sample\nuser-invocable: {user}\ndisable-model-invocation: {disabled}\n---\nBody."
            );
            let parsed = parse_skill_md(&content).unwrap();
            assert_eq!(
                (parsed.user_invocable, parsed.disable_model_invocation),
                (user, disabled)
            );
            let result = validate_skill_md(&content);
            assert!(result.valid);
            assert!(result.errors.is_empty());
            let expected = if !user && disabled {
                vec![
                    "Skill is unreachable: user-invocable is false and disable-model-invocation is true. Neither users nor the model can invoke this skill.",
                ]
            } else {
                vec![]
            };
            assert_eq!(result.warnings, expected);
        }
    }

    #[test]
    fn skill_wire_values_and_unknown_string_fallbacks_are_explicit() {
        for (value, wire) in [
            (SkillSourceType::Markdown, "markdown"),
            (SkillSourceType::Archive, "archive"),
        ] {
            assert_eq!(value.to_string(), wire);
            assert_eq!(
                serde_json::to_value(&value).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(
                serde_json::from_value::<SkillSourceType>(serde_json::json!(wire)).unwrap(),
                value
            );
            assert_eq!(SkillSourceType::from(wire), value);
        }
        for (value, wire) in [
            (SkillStatus::Active, "active"),
            (SkillStatus::Disabled, "disabled"),
            (SkillStatus::Archived, "archived"),
            (SkillStatus::Deleted, "deleted"),
        ] {
            assert_eq!(value.to_string(), wire);
            assert_eq!(
                serde_json::to_value(&value).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(
                serde_json::from_value::<SkillStatus>(serde_json::json!(wire)).unwrap(),
                value
            );
            assert_eq!(SkillStatus::from(wire), value);
        }
        for (value, wire) in [
            (SkillContext::Inline, "inline"),
            (SkillContext::Fork, "fork"),
        ] {
            assert_eq!(value.to_string(), wire);
            assert_eq!(
                serde_json::to_value(&value).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(
                serde_json::from_value::<SkillContext>(serde_json::json!(wire)).unwrap(),
                value
            );
        }
        assert_eq!(SkillSourceType::from("other"), SkillSourceType::Markdown);
        assert_eq!(SkillStatus::from("other"), SkillStatus::Active);
        assert!(serde_json::from_str::<SkillSourceType>("\"other\"").is_err());
        assert!(serde_json::from_str::<SkillStatus>("\"other\"").is_err());
        assert!(serde_json::from_str::<SkillContext>("\"other\"").is_err());
    }

    #[test]
    fn activation_vars_replace_known_occurrences_and_preserve_other_text() {
        for dir in ["/home/user/skills/my-skill", "/.agents/skills/my-skill"] {
            assert_eq!(
                substitute_activation_vars(
                    "${SKILL_DIR}/run ${SESSION_ID} ${SESSION_ID} ${OTHER}",
                    "session_abc",
                    dir
                ),
                format!("{dir}/run session_abc session_abc ${{OTHER}}")
            );
            assert_eq!(
                substitute_activation_vars("No variables. $SESSION_ID", "session_x", dir),
                "No variables. $SESSION_ID"
            );
        }
    }

    #[test]
    fn parser_accepts_literal_byte_limits_and_rejects_the_next_byte() {
        for (field, limit) in [
            ("description", 1024),
            ("license", 500),
            ("compatibility", 500),
            ("argument-hint", 128),
        ] {
            for (value, valid) in [
                ("é".repeat(limit / 2), true),
                (format!("{}x", "é".repeat(limit / 2)), false),
            ] {
                let description = if field == "description" {
                    String::new()
                } else {
                    "description: sample\n".into()
                };
                let content =
                    format!("---\nname: sample\n{description}{field}: {value}\n---\nBody.");
                let result = parse_skill_md(&content);
                assert_eq!(result.is_ok(), valid, "{field}");
                if !valid {
                    assert_eq!(
                        result.unwrap_err(),
                        vec![format!("{field}: exceeds {limit} character limit")]
                    );
                }
            }
        }
        assert!(validate_skill_name(&"a".repeat(64)).is_ok());
        assert_eq!(
            validate_skill_name(&"a".repeat(65)).unwrap_err(),
            vec!["name: must be 1-64 characters"]
        );
        for (size, valid) in [(102400, true), (102401, false)] {
            let result = parse_skill_md(&format!(
                "---\nname: sample\ndescription: sample\n---\n{}",
                "x".repeat(size)
            ));
            assert_eq!(result.is_ok(), valid);
            if !valid {
                assert_eq!(
                    result.unwrap_err(),
                    vec!["instructions: exceeds 100 KB limit"]
                );
            }
        }
    }

    #[test]
    fn validation_line_warning_starts_after_five_hundred_lines() {
        for lines in [500, 501] {
            let result = validate_skill_md(&format!(
                "---\nname: sample\ndescription: sample\n---\n{}",
                "line\n".repeat(lines)
            ));
            assert!(result.valid);
            assert!(result.errors.is_empty());
            let expected = if lines == 501 {
                vec![
                    "Instructions exceed 500 lines (501 lines). Consider splitting into references.",
                ]
            } else {
                vec![]
            };
            assert_eq!(result.warnings, expected);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn command_preprocessing_bounds_fanout_and_preserves_result_order() {
        use std::sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        struct Executor {
            active: AtomicUsize,
            peak: AtomicUsize,
            calls: Mutex<Vec<usize>>,
        }
        #[async_trait::async_trait]
        impl CommandExecutor for Executor {
            async fn execute_command(&self, command: &str) -> CommandResult {
                let index: usize = command.parse().unwrap();
                self.calls.lock().unwrap().push(index);
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(active, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis((35 - index) as u64)).await;
                self.active.fetch_sub(1, Ordering::SeqCst);
                CommandResult {
                    stdout: format!("value-{index}\n"),
                    exit_code: 0,
                }
            }
        }
        let executor = Executor {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            calls: Mutex::new(vec![]),
        };
        let content = (0..34)
            .map(|i| format!("界!`{i}`"))
            .collect::<Vec<_>>()
            .join("|");
        let expected = (0..34)
            .map(|i| {
                if i < 32 {
                    format!("界value-{i}")
                } else {
                    "界[Too many command placeholders: limit is 32]".into()
                }
            })
            .collect::<Vec<_>>()
            .join("|");
        assert_eq!(
            preprocess_command_injections(&content, &executor).await,
            expected
        );
        assert_eq!(executor.peak.load(Ordering::SeqCst), 4);
        assert_eq!(executor.active.load(Ordering::SeqCst), 0);
        let mut calls = executor.calls.into_inner().unwrap();
        calls.sort();
        assert_eq!(calls, (0..32).collect::<Vec<_>>());
    }
}
