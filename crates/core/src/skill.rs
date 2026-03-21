// Skill domain types and SKILL.md parser
//
// Skills are portable instruction packages following the agentskills.io format.
// A skill consists of a SKILL.md file (YAML frontmatter + markdown body)
// with optional bundled scripts, references, and assets.

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

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
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "skill_01933b5a00007000800000000000001"))]
    pub id: SkillId,
    #[cfg_attr(feature = "openapi", schema(example = "pdf-processing"))]
    pub name: String,
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Extract text and tables from PDF files.")
    )]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
    pub source_type: SkillSourceType,
    pub status: SkillStatus,
    pub version: String,
    /// Whether this skill appears as a /slash command for users
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    /// Whether the model is prevented from auto-invoking this skill
    #[serde(default)]
    pub disable_model_invocation: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
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
    /// Hint string for autocomplete (e.g., "<issue-number>")
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
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
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
                    "SKILL.md YAML frontmatter required lenient parsing (strict error: {strict_err}). \
                     Skill authors should fix their YAML."
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
/// - Strip invalid control characters (except newline/tab)
fn try_lenient_yaml_parse(frontmatter: &str) -> Result<SkillFrontmatter, serde_yaml::Error> {
    let fixed = fix_yaml_values(frontmatter);
    serde_yaml::from_str(&fixed)
}

/// Auto-quote YAML values that contain problematic characters.
///
/// For each line that looks like `key: value`, if the value is not already
/// quoted and contains characters that break strict YAML parsing (`:`, `{`,
/// `}`, `[`, `]`, `#`), wrap it in double quotes (escaping inner quotes).
/// Also strips control characters (except `\n`, `\r`, `\t`).
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

                // If value contains problematic chars, quote it
                if value.contains(problematic_chars) {
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
    for c in raw.chars() {
        match (c, in_quote) {
            ('"' | '\'', None) => in_quote = Some(c),
            (q, Some(open)) if q == open => in_quote = None,
            (c, Some(_)) => current.push(c),
            (c, None) if c.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            (c, None) => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// Expand positional argument placeholders in skill content.
///
/// Substitution variables (processed in this order):
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
    let mut result = content.to_string();
    let mut had_placeholder = false;

    // 1. Replace $ARGUMENTS[N] (must be before $ARGUMENTS to avoid partial match)
    let indexed_re = regex::Regex::new(r"\$ARGUMENTS\[([0-9]+)\]").unwrap();
    if indexed_re.is_match(&result) {
        had_placeholder = true;
        result = indexed_re
            .replace_all(&result, |caps: &regex::Captures| {
                let idx: usize = caps[1].parse().unwrap_or(usize::MAX);
                args.get(idx).cloned().unwrap_or_default()
            })
            .to_string();
    }

    // 2. Replace $ARGUMENTS (full string)
    if result.contains("$ARGUMENTS") {
        had_placeholder = true;
        result = result.replace("$ARGUMENTS", raw_args);
    }

    // 3. Replace $N shorthand (single digit, not followed by word chars)
    let chars: Vec<char> = result.chars().collect();
    let mut new_result = String::with_capacity(result.len());
    let mut found_shorthand = false;
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            let digit = chars[i + 1];
            let next_is_word = i + 2 < chars.len()
                && (chars[i + 2].is_ascii_alphanumeric() || chars[i + 2] == '_');

            if !next_is_word {
                found_shorthand = true;
                let idx = (digit as u8 - b'0') as usize;
                new_result.push_str(args.get(idx).map(|s| s.as_str()).unwrap_or(""));
            } else {
                new_result.push('$');
                new_result.push(digit);
            }
            i += 2;
        } else {
            new_result.push(chars[i]);
            i += 1;
        }
    }

    if found_shorthand {
        had_placeholder = true;
        result = new_result;
    }

    // Fallback: append if no placeholders found
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

/// Result of executing a shell command during skill preprocessing.
pub struct CommandResult {
    pub stdout: String,
    pub exit_code: i32,
}

/// Trait for executing shell commands during skill preprocessing.
///
/// Commands in `!`...`` syntax are executed before the skill content
/// is sent to the model, replacing each placeholder with command output.
#[async_trait::async_trait]
pub trait CommandExecutor: Send + Sync {
    async fn execute_command(&self, command: &str) -> CommandResult;
}

/// Default executor using `tokio::process::Command` with bash.
pub struct ProcessCommandExecutor {
    /// Timeout per command in seconds (default: 30).
    pub timeout_secs: u64,
}

impl Default for ProcessCommandExecutor {
    fn default() -> Self {
        Self { timeout_secs: 30 }
    }
}

#[async_trait::async_trait]
impl CommandExecutor for ProcessCommandExecutor {
    async fn execute_command(&self, command: &str) -> CommandResult {
        let timeout = std::time::Duration::from_secs(self.timeout_secs);
        let child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match child {
            Ok(c) => c,
            Err(_) => {
                return CommandResult {
                    stdout: String::new(),
                    exit_code: -1,
                };
            }
        };

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => CommandResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
            },
            Ok(Err(_)) => CommandResult {
                stdout: String::new(),
                exit_code: -1,
            },
            Err(_) => CommandResult {
                stdout: format!(
                    "[Command timed out after {}s: {command}]",
                    self.timeout_secs
                ),
                exit_code: -1,
            },
        }
    }
}

/// Preprocess `!`command`` placeholders in skill content.
///
/// Each `!`command`` is executed via the provided executor and replaced with
/// its stdout. Multiple commands are executed concurrently.
///
/// Substitution pipeline order (caller is responsible for prior steps):
/// 1. `$ARGUMENTS` / `$N` substitution (sync)
/// 2. `${SESSION_ID}` / `${SKILL_DIR}` env substitution (sync)
/// 3. `!`command`` preprocessing (async) — this function
pub async fn preprocess_command_injections(
    content: &str,
    executor: &dyn CommandExecutor,
) -> String {
    let re = Regex::new(r"!`([^`]+)`").unwrap();

    // Collect all matches (command text + byte range)
    let matches: Vec<(String, std::ops::Range<usize>)> = re
        .captures_iter(content)
        .map(|cap| {
            let full = cap.get(0).unwrap();
            let cmd = cap[1].to_string();
            (cmd, full.start()..full.end())
        })
        .collect();

    if matches.is_empty() {
        return content.to_string();
    }

    // Execute all commands concurrently
    let futures: Vec<_> = matches
        .iter()
        .map(|(cmd, _)| executor.execute_command(cmd))
        .collect();
    let results = futures::future::join_all(futures).await;

    // Replace matches in reverse order to preserve byte offsets
    let mut result = content.to_string();
    for ((cmd, range), cmd_result) in matches.iter().zip(results.iter()).rev() {
        let replacement = if cmd_result.exit_code != 0 && cmd_result.stdout.starts_with('[') {
            // Timeout or spawn failure — message is already formatted
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
        assert!(parsed.instructions.contains("# PDF Processing"));
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
    fn test_parse_user_invocable_default_true() {
        let content = r#"---
name: my-skill
description: A skill without explicit invocable field.
---

Instructions.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert!(
            parsed.user_invocable,
            "user_invocable should default to true"
        );
    }

    #[test]
    fn test_parse_user_invocable_explicit_true() {
        let content = r#"---
name: my-skill
description: An invocable skill.
user-invocable: true
---

Instructions.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert!(parsed.user_invocable);
    }

    #[test]
    fn test_parse_user_invocable_false() {
        let content = r#"---
name: background-context
description: Context the agent should know but not a user command.
user-invocable: false
---

Instructions.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert!(!parsed.user_invocable);
    }

    #[test]
    fn test_parse_disable_model_invocation_default_false() {
        let content = r#"---
name: my-skill
description: A skill without disable-model-invocation field.
---

Instructions.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert!(
            !parsed.disable_model_invocation,
            "disable_model_invocation should default to false"
        );
    }

    #[test]
    fn test_parse_disable_model_invocation_true() {
        let content = r#"---
name: manual-only
description: A skill that cannot be auto-invoked by the model.
disable-model-invocation: true
---

Instructions.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert!(parsed.disable_model_invocation);
        assert!(parsed.user_invocable); // default true
    }

    #[test]
    fn test_validate_warns_unreachable_skill() {
        let content = r#"---
name: unreachable
description: Neither user nor model can invoke.
user-invocable: false
disable-model-invocation: true
---

Instructions.
"#;
        let result = validate_skill_md(content);
        assert!(result.valid);
        assert!(
            result.warnings.iter().any(|w| w.contains("unreachable")),
            "Should warn about unreachable skill"
        );
    }

    #[test]
    fn test_skill_source_type_display() {
        assert_eq!(SkillSourceType::Markdown.to_string(), "markdown");
        assert_eq!(SkillSourceType::Archive.to_string(), "archive");
    }

    #[test]
    fn test_skill_status_display() {
        assert_eq!(SkillStatus::Active.to_string(), "active");
        assert_eq!(SkillStatus::Disabled.to_string(), "disabled");
    }

    #[test]
    fn test_skill_source_type_from_str() {
        assert_eq!(SkillSourceType::from("archive"), SkillSourceType::Archive);
        assert_eq!(SkillSourceType::from("markdown"), SkillSourceType::Markdown);
        assert_eq!(SkillSourceType::from("other"), SkillSourceType::Markdown);
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

    #[test]
    fn test_parse_argument_hint_default_none() {
        let content = r#"---
name: my-skill
description: A skill.
---

Body.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert!(parsed.argument_hint.is_none());
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
    fn test_parse_context_default_inline() {
        let content = r#"---
name: my-skill
description: A skill.
---

Body.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.context, SkillContext::Inline);
        assert!(parsed.agent.is_none());
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

    #[test]
    fn test_skill_context_display() {
        assert_eq!(SkillContext::Inline.to_string(), "inline");
        assert_eq!(SkillContext::Fork.to_string(), "fork");
    }

    #[test]
    fn test_skill_context_default() {
        assert_eq!(SkillContext::default(), SkillContext::Inline);
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
model: gpt-4o
---

Body.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("gpt-4o"));
        assert_eq!(parsed.context, SkillContext::Inline);
    }

    #[test]
    fn test_parse_no_model_field() {
        let content = r#"---
name: my-skill
description: A skill.
---

Body.
"#;
        let parsed = parse_skill_md(content).unwrap();
        assert!(parsed.model.is_none());
    }

    #[test]
    fn test_validate_warns_model_without_fork() {
        let content = r#"---
name: my-skill
description: A skill.
model: gpt-4o
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

    #[test]
    fn test_substitute_session_id() {
        let content = "Session: ${SESSION_ID}";
        let result = substitute_activation_vars(content, "session_01abc123", "/some/dir");
        assert_eq!(result, "Session: session_01abc123");
    }

    #[test]
    fn test_substitute_skill_dir_filesystem() {
        let content = "Dir: ${SKILL_DIR}";
        let result = substitute_activation_vars(content, "session_x", "/home/user/skills/my-skill");
        assert_eq!(result, "Dir: /home/user/skills/my-skill");
    }

    #[test]
    fn test_substitute_skill_dir_db_backed() {
        let content = "Dir: ${SKILL_DIR}";
        let result = substitute_activation_vars(content, "session_x", "/.agents/skills/my-skill");
        assert_eq!(result, "Dir: /.agents/skills/my-skill");
    }

    #[test]
    fn test_substitute_both_vars() {
        let content = "Run: ${SKILL_DIR}/run.sh --session ${SESSION_ID}";
        let result =
            substitute_activation_vars(content, "session_01abc", "/.agents/skills/data-tool");
        assert_eq!(
            result,
            "Run: /.agents/skills/data-tool/run.sh --session session_01abc"
        );
    }

    #[test]
    fn test_substitute_no_vars() {
        let content = "No variables here.";
        let result = substitute_activation_vars(content, "session_x", "/dir");
        assert_eq!(result, "No variables here.");
    }

    #[test]
    fn test_substitute_multiple_occurrences() {
        let content = "${SESSION_ID} and ${SESSION_ID} again";
        let result = substitute_activation_vars(content, "session_abc", "/dir");
        assert_eq!(result, "session_abc and session_abc again");
    }

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
    async fn test_preprocess_with_process_executor() {
        let exec = ProcessCommandExecutor::default();

        let content = "Result: !`echo hello world`";
        let result = preprocess_command_injections(content, &exec).await;
        assert_eq!(result, "Result: hello world");
    }

    #[tokio::test]
    async fn test_preprocess_command_not_found() {
        let exec = MockExecutor::new(); // No responses registered

        let content = "Result: !`unknown-cmd`";
        let result = preprocess_command_injections(content, &exec).await;
        assert!(result.contains("[Command failed: unknown-cmd"));
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
    fn test_lenient_parse_hash_in_value() {
        let content = "---\nname: my-skill\ndescription: Process C# files\n---\n\nBody.\n";
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.description, "Process C# files");
    }

    #[test]
    fn test_lenient_parse_brackets_in_value() {
        let content =
            "---\nname: my-skill\ndescription: Parse [markdown] and {templates}\n---\n\nBody.\n";
        let parsed = parse_skill_md(content).unwrap();
        assert_eq!(parsed.description, "Parse [markdown] and {templates}");
    }

    #[test]
    fn test_lenient_parse_already_quoted_value_unchanged() {
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
        assert!(fixed.contains("  version: 1.0"));
        assert!(fixed.contains("  key: value: nested"));
    }
}
