//! Portable skill configuration and the SKILL.md parser.
//!
//! Skills contain instructions with optional scripts, references, and assets.
//! Persisted skill records and their lifecycle live in the server.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use tracing::warn;

/// Match argument placeholders in the template, never in inserted argument values.
#[expect(clippy::unwrap_used, reason = "the pattern is a valid literal regex")]
static ARGUMENTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$ARGUMENTS(?:\[([0-9]+)\])?|\$([0-9])").unwrap());

/// Cached regex for ``!`command` `` dynamic command injection syntax.
#[expect(clippy::unwrap_used, reason = "the pattern is a valid literal regex")]
static COMMAND_INJECTION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!`([^`]+)`").unwrap());

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

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
            let matched = caps.get_match();
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
            let full = cap.get_match();
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
#[path = "skill_tests.rs"]
mod tests;
