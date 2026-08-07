// User-defined hooks: shared types.
//
// See `knowledge/runtime-resources/user-hooks.md` for the contract. This module defines the data
// shape — events, matchers, executor spec, outcomes — used both as the
// on-the-wire JSON config and as the in-memory `UserHookSpec` carried through
// the capability collection pipeline. Capability authors return
// `Vec<UserHookSpec>` from `Capability::user_hooks()`; the user-facing
// `user_hooks` capability parses the same shape from its config.
//
// Adapter construction (spec -> Arc<dyn …Hook>) lives in `hook_adapter` and
// the executor backend in `hook_executor`. This module is intentionally pure
// data + validation.

use serde::{Deserialize, Serialize};

// ============================================================================
// HookEvent
// ============================================================================

/// The lifecycle point at which a hook fires.
///
/// Six events. See `knowledge/runtime-resources/user-hooks.md` for semantics and wire payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    TurnEnd,
    SessionEnd,
}

impl HookEvent {
    /// Whether this event carries tool context (i.e. a matcher can apply).
    pub fn supports_matcher(self) -> bool {
        matches!(self, HookEvent::PreToolUse | HookEvent::PostToolUse)
    }

    /// Whether a hook fired on this event can return `Block`.
    pub fn can_block(self) -> bool {
        matches!(self, HookEvent::UserPromptSubmit | HookEvent::PreToolUse)
    }

    /// Stable wire string used in event payloads and audit logs.
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "session_start",
            HookEvent::UserPromptSubmit => "user_prompt_submit",
            HookEvent::PreToolUse => "pre_tool_use",
            HookEvent::PostToolUse => "post_tool_use",
            HookEvent::TurnEnd => "turn_end",
            HookEvent::SessionEnd => "session_end",
        }
    }
}

// ============================================================================
// HookMatcher
// ============================================================================

/// Predicate over a tool call, deciding whether a `pre_tool_use` or
/// `post_tool_use` hook fires for a given invocation.
///
/// All fields are optional. An empty matcher fires on every event of the
/// configured kind. Matchers on non-tool events are rejected at validation
/// time (see [`UserHookSpec::validate`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HookMatcher {
    /// Exact tool name match.
    pub tool_name: Option<String>,
    /// Restricted glob: `a|b|c` alternation or trailing `*`. No general glob.
    pub tool_name_glob: Option<String>,
    /// JSONPath expression evaluated against `ToolCall.arguments`. Only the
    /// `$.field.subfield` dot-path subset is honored. If absent, the matcher
    /// does not examine arguments.
    pub args_jsonpath: Option<String>,
    /// Regex matched against the value at `args_jsonpath`. If absent and
    /// `args_jsonpath` is set, the matcher fires when the path resolves to
    /// any non-empty string.
    pub match_regex: Option<String>,
    /// Inverse: matcher fires when the extracted value matches this regex.
    /// Mutually exclusive with `match_regex`.
    pub deny_regex: Option<String>,
}

impl HookMatcher {
    fn is_empty(&self) -> bool {
        self.tool_name.is_none()
            && self.tool_name_glob.is_none()
            && self.args_jsonpath.is_none()
            && self.match_regex.is_none()
            && self.deny_regex.is_none()
    }

    /// Decide whether this matcher fires on the given tool call.
    ///
    /// `tool_name` is the runtime tool name; `args` is `ToolCall.arguments`.
    /// Returns `true` when the hook should run.
    pub fn matches(&self, tool_name: &str, args: &serde_json::Value) -> bool {
        if self.is_empty() {
            return true;
        }

        if let Some(exact) = &self.tool_name
            && exact != tool_name
        {
            return false;
        }
        if let Some(glob) = &self.tool_name_glob
            && !glob_matches(glob, tool_name)
        {
            return false;
        }

        // Args check.
        let extracted = self
            .args_jsonpath
            .as_deref()
            .map(|path| extract_path(args, path));

        match (&self.match_regex, &self.deny_regex, extracted) {
            (None, None, None) => true,
            (None, None, Some(value)) => !value.is_empty(),
            (Some(re), None, Some(value)) => regex_match(re, &value),
            (None, Some(re), Some(value)) => regex_match(re, &value),
            (Some(_), Some(_), _) => false, // mutual exclusion enforced by validate
            (Some(_), None, None) | (None, Some(_), None) => false,
        }
    }
}

/// Restricted glob match: supports `a|b|c` alternation and trailing `*`.
/// Returns true if the name matches the pattern.
fn glob_matches(pattern: &str, name: &str) -> bool {
    for alt in pattern.split('|') {
        let alt = alt.trim();
        if alt.is_empty() {
            continue;
        }
        if let Some(prefix) = alt.strip_suffix('*') {
            if name.starts_with(prefix) {
                return true;
            }
        } else if alt == name {
            return true;
        }
    }
    false
}

/// Extract a dot-path value (`$.foo.bar`) from a JSON value as a string.
/// Returns the empty string when the path doesn't resolve.
fn extract_path(value: &serde_json::Value, path: &str) -> String {
    let trimmed = path.strip_prefix("$.").unwrap_or(path);
    let mut current = value;
    for segment in trimmed.split('.') {
        if segment.is_empty() {
            continue;
        }
        current = match current.get(segment) {
            Some(v) => v,
            None => return String::new(),
        };
    }
    match current {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn regex_match(pattern: &str, value: &str) -> bool {
    regex::Regex::new(pattern)
        .map(|re| re.is_match(value))
        .unwrap_or(false)
}

// ============================================================================
// ExecutorSpec
// ============================================================================

/// Backend that executes the hook payload. Only `bash` is wired in v1; the
/// enum is open for future webhook/wasm/blueprint backends without breaking
/// existing configs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutorSpec {
    /// Run a shell command inside `bashkit_shell` against the session VFS.
    Bash {
        /// Command line passed to bash. Required.
        command: String,
        /// Extra env vars layered onto the default executor env. Capped at
        /// `MAX_HOOK_ENV_VARS` entries by validation.
        #[serde(default)]
        env: std::collections::BTreeMap<String, String>,
    },
}

pub const MAX_HOOK_ENV_VARS: usize = 16;
pub const MAX_HOOK_COMMAND_BYTES: usize = 16 * 1024;
pub const MAX_HOOK_MATCHER_STRING_BYTES: usize = 2 * 1024;

// ============================================================================
// OnError
// ============================================================================

/// How the runtime treats an executor failure (timeout, non-JSON output,
/// sandbox error).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// Treat as `Block` — abort the action this hook was protecting.
    Block,
    /// Log + continue.
    Allow,
    /// Log + emit `hook.warning` event + continue.
    #[default]
    Warn,
}

// ============================================================================
// HookId, HookSource
// ============================================================================

/// Stable identifier for a hook entry. Surfaced in audit logs and used by the
/// `user_hooks` capability's `disabled_contributions` list.
///
/// Format:
/// - Capability contribution: `{capability_id}:{name}`
/// - User config: `user:{name}`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HookId(pub String);

impl HookId {
    pub fn for_capability(capability_id: &str, name: &str) -> Self {
        Self(format!("{capability_id}:{name}"))
    }
    pub fn for_user(name: &str) -> Self {
        Self(format!("user:{name}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who contributed this hook spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookSource {
    UserConfig,
    Capability { capability_id: String },
}

// ============================================================================
// UserHookSpec
// ============================================================================

/// User-facing hook specification. This is the type capability authors
/// return from `Capability::user_hooks()` and the type the `user_hooks`
/// capability parses from its config.
///
/// Validation is performed once, at capability collection time, by
/// `HookAdapterBuilder` (see follow-up module). Validation enforces:
///
/// - timeout bounds (100..=30_000 ms)
/// - matcher present only on tool events
/// - env var count cap
/// - regex compilability
/// - mutual exclusion of `match_regex` and `deny_regex`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserHookSpec {
    /// Defaults to `"{event}_{index}"` when omitted at the user-config layer.
    #[serde(default)]
    pub id: Option<String>,
    pub event: HookEvent,
    #[serde(default)]
    pub matcher: HookMatcher,
    pub executor: ExecutorSpec,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
    #[serde(default)]
    pub on_error: OnError,
    #[serde(default)]
    pub description: Option<String>,
    /// Populated by `HookAdapterBuilder`; defaults to `UserConfig` when the
    /// spec arrives via deserialization.
    #[serde(default = "default_source")]
    pub source: HookSource,
}

fn default_timeout_ms() -> u32 {
    5000
}
fn default_source() -> HookSource {
    HookSource::UserConfig
}

pub const MIN_HOOK_TIMEOUT_MS: u32 = 100;
pub const MAX_HOOK_TIMEOUT_MS: u32 = 30_000;

#[derive(Debug, thiserror::Error)]
pub enum HookSpecError {
    #[error("hook timeout_ms {0} is out of range ({MIN_HOOK_TIMEOUT_MS}..={MAX_HOOK_TIMEOUT_MS})")]
    TimeoutOutOfRange(u32),
    #[error("hook event `{0}` does not accept a matcher")]
    MatcherOnNonToolEvent(&'static str),
    #[error("hook has more than {MAX_HOOK_ENV_VARS} env vars")]
    TooManyEnvVars,
    #[error("hook bash command is longer than {MAX_HOOK_COMMAND_BYTES} bytes")]
    CommandTooLong,
    #[error("hook matcher field `{0}` is longer than {MAX_HOOK_MATCHER_STRING_BYTES} bytes")]
    MatcherFieldTooLong(&'static str),
    #[error("hook has both match_regex and deny_regex set; choose one")]
    AmbiguousRegex,
    #[error("hook regex `{0}` failed to compile: {1}")]
    InvalidRegex(String, regex::Error),
    #[error("hook bash command is empty")]
    EmptyCommand,
    #[error("hook tool_name_glob `{0}` contains unsupported syntax")]
    UnsupportedGlob(String),
}

impl UserHookSpec {
    /// Resolve the spec's stable `HookId`, given its position in the source
    /// (used to derive a default name when the user omitted one).
    pub fn resolve_id(&self, index: usize) -> HookId {
        let name = self
            .id
            .clone()
            .unwrap_or_else(|| format!("{}_{}", self.event.as_str(), index));
        match &self.source {
            HookSource::UserConfig => HookId::for_user(&name),
            HookSource::Capability { capability_id } => {
                HookId::for_capability(capability_id, &name)
            }
        }
    }

    /// Validate the spec against the global rules listed on the struct.
    /// Called by `HookAdapterBuilder`; capability authors don't invoke it
    /// directly.
    pub fn validate(&self) -> Result<(), HookSpecError> {
        if !(MIN_HOOK_TIMEOUT_MS..=MAX_HOOK_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(HookSpecError::TimeoutOutOfRange(self.timeout_ms));
        }

        if !self.event.supports_matcher() && !self.matcher_is_empty() {
            return Err(HookSpecError::MatcherOnNonToolEvent(self.event.as_str()));
        }

        match &self.executor {
            ExecutorSpec::Bash { command, env } => {
                if command.trim().is_empty() {
                    return Err(HookSpecError::EmptyCommand);
                }
                if command.len() > MAX_HOOK_COMMAND_BYTES {
                    return Err(HookSpecError::CommandTooLong);
                }
                if env.len() > MAX_HOOK_ENV_VARS {
                    return Err(HookSpecError::TooManyEnvVars);
                }
            }
        }

        validate_optional_matcher_string("tool_name", &self.matcher.tool_name)?;
        validate_optional_matcher_string("tool_name_glob", &self.matcher.tool_name_glob)?;
        validate_optional_matcher_string("args_jsonpath", &self.matcher.args_jsonpath)?;
        validate_optional_matcher_string("match_regex", &self.matcher.match_regex)?;
        validate_optional_matcher_string("deny_regex", &self.matcher.deny_regex)?;

        if self.matcher.match_regex.is_some() && self.matcher.deny_regex.is_some() {
            return Err(HookSpecError::AmbiguousRegex);
        }
        if let Some(re) = &self.matcher.match_regex {
            regex::Regex::new(re).map_err(|e| HookSpecError::InvalidRegex(re.clone(), e))?;
        }
        if let Some(re) = &self.matcher.deny_regex {
            regex::Regex::new(re).map_err(|e| HookSpecError::InvalidRegex(re.clone(), e))?;
        }
        if let Some(glob) = &self.matcher.tool_name_glob {
            validate_glob(glob)?;
        }

        Ok(())
    }

    fn matcher_is_empty(&self) -> bool {
        self.matcher.is_empty()
    }
}

fn validate_optional_matcher_string(
    field: &'static str,
    value: &Option<String>,
) -> Result<(), HookSpecError> {
    if value
        .as_ref()
        .is_some_and(|value| value.len() > MAX_HOOK_MATCHER_STRING_BYTES)
    {
        return Err(HookSpecError::MatcherFieldTooLong(field));
    }
    Ok(())
}

fn validate_glob(glob: &str) -> Result<(), HookSpecError> {
    // Allowed syntax: `a|b|c` alternation, trailing `*` per alternative.
    // Reject any other glob metachar.
    for alt in glob.split('|') {
        let alt = alt.trim_end_matches('*');
        if alt
            .chars()
            .any(|c| matches!(c, '*' | '?' | '[' | ']' | '{' | '}'))
        {
            return Err(HookSpecError::UnsupportedGlob(glob.to_string()));
        }
    }
    Ok(())
}

// ============================================================================
// HookOutcome
// ============================================================================

/// Result of running a hook's executor against a single payload.
#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// Hook ran cleanly with no requested change.
    Allow,
    /// Hook ran cleanly and asks the runtime to apply this JSON patch to the
    /// event subject. Patch shape is event-specific (see spec).
    Mutate {
        patch: serde_json::Value,
        reason: Option<String>,
    },
    /// Hook asks the runtime to abort the current action.
    Block {
        reason: String,
        user_message: Option<String>,
    },
    /// Executor failed (timeout, non-JSON output, sandbox error). The
    /// adapter applies the `on_error` policy to decide what to do.
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bash_exec() -> ExecutorSpec {
        ExecutorSpec::Bash {
            command: "echo hi".to_string(),
            env: Default::default(),
        }
    }

    #[test]
    fn glob_alternation_matches() {
        assert!(glob_matches("bash|edit_file", "bash"));
        assert!(glob_matches("bash|edit_file", "edit_file"));
        assert!(!glob_matches("bash|edit_file", "read_file"));
    }

    #[test]
    fn glob_trailing_star_matches_prefix() {
        assert!(glob_matches("daytona_*", "daytona_exec"));
        assert!(glob_matches("daytona_*", "daytona_read_file"));
        assert!(!glob_matches("daytona_*", "bashkit_shell"));
    }

    #[test]
    fn empty_matcher_matches_anything() {
        let m = HookMatcher::default();
        assert!(m.matches("anything", &json!({})));
    }

    #[test]
    fn matcher_exact_tool_name() {
        let m = HookMatcher {
            tool_name: Some("edit_file".into()),
            ..Default::default()
        };
        assert!(m.matches("edit_file", &json!({})));
        assert!(!m.matches("read_file", &json!({})));
    }

    #[test]
    fn matcher_args_jsonpath_extract() {
        let m = HookMatcher {
            args_jsonpath: Some("$.command".into()),
            ..Default::default()
        };
        // Path resolves to non-empty -> match.
        assert!(m.matches("bash", &json!({"command": "ls"})));
        // Path missing -> no match.
        assert!(!m.matches("bash", &json!({"other": "ls"})));
    }

    #[test]
    fn matcher_deny_regex_blocks() {
        let m = HookMatcher {
            args_jsonpath: Some("$.command".into()),
            deny_regex: Some(r"^rm -rf".into()),
            ..Default::default()
        };
        assert!(m.matches("bash", &json!({"command": "rm -rf /"})));
        assert!(!m.matches("bash", &json!({"command": "ls"})));
    }

    #[test]
    fn matcher_both_regex_kinds_rejected_at_match_time() {
        let m = HookMatcher {
            args_jsonpath: Some("$.command".into()),
            match_regex: Some("a".into()),
            deny_regex: Some("b".into()),
            ..Default::default()
        };
        // Defensive: matches() returns false; validate() rejects.
        assert!(!m.matches("bash", &json!({"command": "ab"})));
    }

    #[test]
    fn validate_timeout_bounds() {
        let mut spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher::default(),
            executor: bash_exec(),
            timeout_ms: 50,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::TimeoutOutOfRange(50))
        ));
        spec.timeout_ms = 60_000;
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::TimeoutOutOfRange(60_000))
        ));
        spec.timeout_ms = 5_000;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn validate_matcher_on_non_tool_event_rejected() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::SessionStart,
            matcher: HookMatcher {
                tool_name: Some("edit_file".into()),
                ..Default::default()
            },
            executor: bash_exec(),
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::MatcherOnNonToolEvent("session_start"))
        ));
    }

    #[test]
    fn validate_ambiguous_regex_rejected() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher {
                args_jsonpath: Some("$.x".into()),
                match_regex: Some("a".into()),
                deny_regex: Some("b".into()),
                ..Default::default()
            },
            executor: bash_exec(),
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::AmbiguousRegex)
        ));
    }

    #[test]
    fn validate_invalid_regex_rejected() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher {
                args_jsonpath: Some("$.x".into()),
                match_regex: Some("(".into()),
                ..Default::default()
            },
            executor: bash_exec(),
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::InvalidRegex(_, _))
        ));
    }

    #[test]
    fn validate_empty_command_rejected() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher::default(),
            executor: ExecutorSpec::Bash {
                command: "   ".to_string(),
                env: Default::default(),
            },
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(spec.validate(), Err(HookSpecError::EmptyCommand)));
    }

    #[test]
    fn validate_oversized_command_rejected() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher::default(),
            executor: ExecutorSpec::Bash {
                command: "x".repeat(MAX_HOOK_COMMAND_BYTES + 1),
                env: Default::default(),
            },
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::CommandTooLong)
        ));
    }

    #[test]
    fn validate_oversized_matcher_regex_rejected_before_compile() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher {
                match_regex: Some("[".repeat(MAX_HOOK_MATCHER_STRING_BYTES + 1)),
                ..Default::default()
            },
            executor: bash_exec(),
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::MatcherFieldTooLong("match_regex"))
        ));
    }

    #[test]
    fn validate_too_many_env_vars_rejected() {
        let mut env = std::collections::BTreeMap::new();
        for i in 0..(MAX_HOOK_ENV_VARS + 1) {
            env.insert(format!("VAR_{i}"), "x".into());
        }
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher::default(),
            executor: ExecutorSpec::Bash {
                command: "true".into(),
                env,
            },
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::TooManyEnvVars)
        ));
    }

    #[test]
    fn validate_glob_rejects_unsupported_syntax() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher {
                tool_name_glob: Some("[abc]*".into()),
                ..Default::default()
            },
            executor: bash_exec(),
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert!(matches!(
            spec.validate(),
            Err(HookSpecError::UnsupportedGlob(_))
        ));
    }

    #[test]
    fn resolve_id_defaults_to_event_index_for_user() {
        let spec = UserHookSpec {
            id: None,
            event: HookEvent::PreToolUse,
            matcher: HookMatcher::default(),
            executor: bash_exec(),
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::UserConfig,
        };
        assert_eq!(spec.resolve_id(2).as_str(), "user:pre_tool_use_2");
    }

    #[test]
    fn resolve_id_namespaces_capability_contributions() {
        let spec = UserHookSpec {
            id: Some("fmt".into()),
            event: HookEvent::PostToolUse,
            matcher: HookMatcher::default(),
            executor: bash_exec(),
            timeout_ms: 5000,
            on_error: OnError::Warn,
            description: None,
            source: HookSource::Capability {
                capability_id: "rust_quality_pack".into(),
            },
        };
        assert_eq!(spec.resolve_id(0).as_str(), "rust_quality_pack:fmt");
    }

    #[test]
    fn event_block_capability() {
        assert!(HookEvent::PreToolUse.can_block());
        assert!(HookEvent::UserPromptSubmit.can_block());
        assert!(!HookEvent::PostToolUse.can_block());
        assert!(!HookEvent::SessionStart.can_block());
    }

    #[test]
    fn event_matcher_support() {
        assert!(HookEvent::PreToolUse.supports_matcher());
        assert!(HookEvent::PostToolUse.supports_matcher());
        assert!(!HookEvent::SessionStart.supports_matcher());
        assert!(!HookEvent::UserPromptSubmit.supports_matcher());
    }

    #[test]
    fn deserialize_full_spec_from_user_config_json() {
        let raw = serde_json::json!({
            "id": "fmt",
            "event": "post_tool_use",
            "matcher": { "tool_name": "edit_file" },
            "executor": { "type": "bash", "command": "scripts/fmt.sh" },
            "timeout_ms": 5000,
            "on_error": "warn",
            "description": "format after edit"
        });
        let spec: UserHookSpec = serde_json::from_value(raw).unwrap();
        assert_eq!(spec.event, HookEvent::PostToolUse);
        assert_eq!(spec.matcher.tool_name.as_deref(), Some("edit_file"));
        assert!(matches!(spec.executor, ExecutorSpec::Bash { .. }));
        assert!(matches!(spec.source, HookSource::UserConfig));
    }

    #[test]
    fn deserialize_minimal_spec_applies_defaults() {
        let raw = serde_json::json!({
            "event": "pre_tool_use",
            "executor": { "type": "bash", "command": "true" }
        });
        let spec: UserHookSpec = serde_json::from_value(raw).unwrap();
        assert_eq!(spec.timeout_ms, 5000);
        assert!(matches!(spec.on_error, OnError::Warn));
        assert!(spec.matcher_is_empty());
    }
}
