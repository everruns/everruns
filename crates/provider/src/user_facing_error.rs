use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

pub mod codes {
    pub const BUDGET_EXHAUSTED: &str = "budget_exhausted";
    pub const BUDGET_PAUSED: &str = "budget_paused";
    pub const MODEL_UNAVAILABLE: &str = "model_unavailable";
    pub const MODEL_NOT_CONFIGURED: &str = "model_not_configured";
    pub const REQUEST_TOO_LARGE: &str = "request_too_large";
    pub const PROVIDER_RATE_LIMITED: &str = "provider_rate_limited";
    /// Subscription/plan usage limit was reached (e.g. ChatGPT/Codex
    /// `usage_limit_reached`). Distinct from `provider_rate_limited` (a short
    /// transient throttle) because the reset is far in the future (hours) and
    /// carries a concrete `resets_at` timestamp, and distinct from
    /// `provider_quota_exhausted` (billing/credits) because it recovers on its
    /// own at the reset time without operator action.
    pub const PROVIDER_USAGE_LIMIT_REACHED: &str = "provider_usage_limit_reached";
    pub const PROVIDER_MISCONFIGURED: &str = "provider_misconfigured";
    /// Provider account is out of credits/quota (billing). Distinct from
    /// `provider_misconfigured` (bad/missing API key) so operators can tell
    /// "top up the account" apart from "fix the key".
    pub const PROVIDER_QUOTA_EXHAUSTED: &str = "provider_quota_exhausted";
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    pub const PROCESSING_ERROR: &str = "processing_error";
    pub const DEPENDENCY_UNAVAILABLE: &str = "dependency_unavailable";
    pub const INVALID_TOOL_SCHEMA: &str = "invalid_tool_schema";
    pub const MAX_ITERATIONS: &str = "max_iterations";
    pub const SOFT_LIMIT_REACHED: &str = "soft_limit_reached";
    /// A `user_prompt_submit` hook rejected the inbound user message.
    pub const BLOCKED_BY_HOOK: &str = "blocked_by_hook";
}

pub type UserFacingErrorFields = BTreeMap<String, Value>;

/// Message/event metadata keys used to track error disclosure decisions.
pub mod metadata_keys {
    /// Disclosure mode applied when the error surfaced ("generic" | "standard" | "detailed").
    pub const ERROR_DISCLOSURE: &str = "error_disclosure";
    /// The classified error code before disclosure was applied. Differs from
    /// `error_code` only in `generic` mode, where the displayed code collapses
    /// to `processing_error`.
    pub const SOURCE_ERROR_CODE: &str = "source_error_code";
}

/// How much detail about a run-blocking error is shown to session viewers.
///
/// Ordering matters: variants are declared least → most disclosing so that
/// per-message control overrides can be clamped with `min` against the
/// capability-configured ceiling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum ErrorDisclosure {
    /// Collapse every blocking error into one generic, localizable message
    /// (`processing_error`, no fields). For public-facing agents.
    Generic,
    /// Stable error code + structured interpolation fields. Current default.
    #[default]
    Standard,
    /// Standard plus a `detail` field carrying the underlying driver error
    /// text. For trusted surfaces such as coding-agent harnesses.
    Detailed,
}

impl ErrorDisclosure {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "generic" => Some(ErrorDisclosure::Generic),
            "standard" => Some(ErrorDisclosure::Standard),
            "detailed" => Some(ErrorDisclosure::Detailed),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorDisclosure::Generic => "generic",
            ErrorDisclosure::Standard => "standard",
            ErrorDisclosure::Detailed => "detailed",
        }
    }
}

/// Maximum length of the `detail` field attached in `Detailed` mode. Provider
/// error bodies are normally short; this guards against pathological payloads
/// bloating messages and events.
const DETAIL_MAX_CHARS: usize = 1000;

/// Provider quota/billing-exhaustion patterns shared by the string classifier
/// and the driver-boundary semantic classifier (`LlmErrorKind`).
pub fn is_provider_quota_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("insufficient_quota")
        || lower.contains("insufficient quota")
        || lower.contains("exceeded your current quota")
        || lower.contains("credit_balance_exhausted")
        || lower.contains("credit balance is too low")
}

/// Subscription/plan usage-limit patterns shared by the string classifier and
/// the transient-retry gate. These recover only at a future reset time (hours
/// away), so unlike an ordinary 429 they must not be retried within the driver
/// backoff window nor collapsed into the "wait a moment" rate-limit copy.
///
/// The canonical shape is the ChatGPT/Codex `429` body
/// (`{"error":{"type":"usage_limit_reached", ...}}`), but the match is kept
/// provider-agnostic so any driver surfacing the same wording is covered.
pub fn is_usage_limit_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("usage_limit_reached")
        || lower.contains("usage limit reached")
        || lower.contains("usage limit has been reached")
}

/// Extract the absolute reset time (`resets_at`, unix seconds) from a usage-limit
/// error body when present. Prefers the absolute `resets_at` field over the
/// relative `resets_in_seconds` because this classifier is clock-free and callers
/// want a stable timestamp they can render in the viewer's timezone.
pub fn parse_usage_limit_reset_at(message: &str) -> Option<i64> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#""resets_at"\s*:\s*(?P<resets_at>\d{9,})"#).expect("valid resets_at regex")
    });
    re.captures(message)?
        .name("resets_at")?
        .as_str()
        .parse::<i64>()
        .ok()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UserFacingError {
    pub code: String,
    #[serde(default, skip_serializing_if = "UserFacingErrorFields::is_empty")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub fields: UserFacingErrorFields,
}

#[derive(Debug, Clone, Default)]
pub struct UserFacingErrorContext {
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub retry_after: Option<u64>,
}

impl UserFacingErrorContext {
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_retry_after(mut self, retry_after: u64) -> Self {
        self.retry_after = Some(retry_after);
        self
    }
}

impl UserFacingError {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            fields: UserFacingErrorFields::new(),
        }
    }

    pub fn with_field<T: Serialize>(mut self, key: impl Into<String>, value: T) -> Self {
        let value = serde_json::to_value(value).unwrap_or(Value::Null);
        if !value.is_null() {
            self.fields.insert(key.into(), value);
        }
        self
    }

    pub fn with_optional_field<T: Serialize>(
        self,
        key: impl Into<String>,
        value: Option<T>,
    ) -> Self {
        match value {
            Some(value) => self.with_field(key, value),
            None => self,
        }
    }

    pub fn error_fields(&self) -> Option<UserFacingErrorFields> {
        (!self.fields.is_empty()).then_some(self.fields.clone())
    }

    pub fn apply_to_event_fields(
        &self,
        error_code: &mut Option<String>,
        error_fields: &mut Option<UserFacingErrorFields>,
    ) {
        *error_code = Some(self.code.clone());
        *error_fields = self.error_fields();
    }

    pub fn apply_to_message_metadata(&self, metadata: &mut HashMap<String, Value>) {
        metadata.insert("error_code".to_string(), Value::String(self.code.clone()));
        if let Some(fields) = self.error_fields() {
            metadata.insert(
                "error_fields".to_string(),
                serde_json::to_value(fields).unwrap_or(Value::Null),
            );
        }
    }

    /// Apply an error-disclosure mode, returning the error as it should be
    /// shown to session viewers. The original (source) error stays available
    /// to the caller for tracking metadata.
    ///
    /// - `Generic` collapses to `processing_error` with no fields.
    /// - `Standard` returns the error unchanged.
    /// - `Detailed` attaches `detail` (the underlying driver error text,
    ///   truncated) as an extra interpolation field.
    pub fn apply_disclosure(&self, mode: ErrorDisclosure, detail: Option<&str>) -> UserFacingError {
        match mode {
            ErrorDisclosure::Generic => UserFacingError::new(codes::PROCESSING_ERROR),
            ErrorDisclosure::Standard => self.clone(),
            ErrorDisclosure::Detailed => {
                let detail = detail.map(str::trim).filter(|d| !d.is_empty());
                match detail {
                    Some(detail) => self
                        .clone()
                        .with_field("detail", truncate_chars(detail, DETAIL_MAX_CHARS)),
                    None => self.clone(),
                }
            }
        }
    }

    /// Record disclosure tracking metadata on a message: the mode that was
    /// applied and the pre-disclosure (source) error code.
    pub fn apply_disclosure_to_message_metadata(
        metadata: &mut HashMap<String, Value>,
        mode: ErrorDisclosure,
        source_code: &str,
    ) {
        metadata.insert(
            metadata_keys::ERROR_DISCLOSURE.to_string(),
            Value::String(mode.as_str().to_string()),
        );
        metadata.insert(
            metadata_keys::SOURCE_ERROR_CODE.to_string(),
            Value::String(source_code.to_string()),
        );
    }

    pub fn fallback_message(&self) -> String {
        self.base_fallback_message()
    }

    fn base_fallback_message(&self) -> String {
        match self.code.as_str() {
            codes::BUDGET_EXHAUSTED => budget_exhausted_message(&self.fields),
            codes::BUDGET_PAUSED => budget_paused_message(&self.fields),
            codes::SOFT_LIMIT_REACHED => string_field(&self.fields, "message")
                .unwrap_or("Soft limit reached.")
                .to_string(),
            codes::MODEL_UNAVAILABLE => {
                if let Some(model_id) = string_field(&self.fields, "model_id") {
                    format!(
                        "The model `{}` is not available. It may have been removed, renamed, or your API key may not have access to it. Please select a different model.",
                        model_id
                    )
                } else {
                    "The selected model is not available. Please select a different model."
                        .to_string()
                }
            }
            codes::MODEL_NOT_CONFIGURED => {
                "No model is configured for this chat. Choose a model or configure a default model, then try again."
                    .to_string()
            }
            codes::REQUEST_TOO_LARGE => {
                "The conversation has become too long for the model to process. Please start a new session or reduce the context size.".to_string()
            }
            codes::PROVIDER_RATE_LIMITED => {
                "Rate limited by the AI provider. Please wait a moment.".to_string()
            }
            codes::PROVIDER_USAGE_LIMIT_REACHED => usage_limit_reached_message(&self.fields),
            codes::PROVIDER_MISCONFIGURED => {
                "There is a misconfiguration with the AI provider. Please contact support."
                    .to_string()
            }
            codes::PROVIDER_QUOTA_EXHAUSTED => {
                "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue."
                    .to_string()
            }
            codes::PROVIDER_UNAVAILABLE => {
                "The AI provider is experiencing issues. Please try again shortly.".to_string()
            }
            codes::DEPENDENCY_UNAVAILABLE => {
                "Execution stopped because a required dependency is unavailable.".to_string()
            }
            codes::INVALID_TOOL_SCHEMA => {
                "A connected tool uses an input schema that this model provider does not support. Update the integration or choose a different model provider, then try again."
                    .to_string()
            }
            _ => "I encountered an error while processing your request. Please try again later."
                .to_string(),
        }
    }
}

pub fn classify_runtime_error_message(
    error: &str,
    context: &UserFacingErrorContext,
) -> UserFacingError {
    let normalized = trim_error_chain_prefixes(error).trim();
    let lower = normalized.to_ascii_lowercase();

    if let Some(fields) = parse_budget_exhausted_fields(normalized) {
        return UserFacingError {
            code: codes::BUDGET_EXHAUSTED.to_string(),
            fields,
        };
    }

    if normalized.starts_with("Budget exhausted.") {
        return UserFacingError::new(codes::BUDGET_EXHAUSTED);
    }

    if normalized.starts_with("Budget exhausted (") {
        return UserFacingError::new(codes::BUDGET_EXHAUSTED);
    }

    if let Some(fields) = parse_budget_paused_fields(normalized) {
        return UserFacingError {
            code: codes::BUDGET_PAUSED.to_string(),
            fields,
        };
    }

    if normalized.starts_with("Budget paused.") || normalized.starts_with("Budget paused with ") {
        return UserFacingError::new(codes::BUDGET_PAUSED);
    }

    if normalized.starts_with("Budget paused (") || normalized.starts_with("Soft limit reached.") {
        return if normalized.starts_with("Soft limit reached.") {
            UserFacingError::new(codes::SOFT_LIMIT_REACHED).with_field("message", normalized)
        } else {
            UserFacingError::new(codes::BUDGET_PAUSED)
        };
    }

    if let Some(model_id) = normalized.strip_prefix("Model not available: ") {
        return UserFacingError::new(codes::MODEL_UNAVAILABLE).with_field("model_id", model_id);
    }

    if normalized.starts_with("Model not configured") || lower.contains("no model configured") {
        return UserFacingError::new(codes::MODEL_NOT_CONFIGURED);
    }

    if normalized.starts_with("Request too large:")
        || lower.contains("context length")
        || lower.contains("maximum context length")
    {
        return UserFacingError::new(codes::REQUEST_TOO_LARGE)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone());
    }

    if is_invalid_tool_schema_message(&lower) {
        return UserFacingError::new(codes::INVALID_TOOL_SCHEMA)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone())
            .with_optional_field("schema_path", extract_schema_path(normalized));
    }

    // Exhausted provider billing (OpenAI: HTTP 429 + `insufficient_quota`,
    // Anthropic: 400 + "credit balance is too low"). The "(429)" prefix would
    // otherwise route it to PROVIDER_RATE_LIMITED ("wait a moment"), but the
    // condition is non-transient and needs operator action (top up the
    // account or raise limits), so it gets its own code.
    if is_provider_quota_message(normalized) {
        return UserFacingError::new(codes::PROVIDER_QUOTA_EXHAUSTED)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone());
    }

    // Subscription/plan usage limit (e.g. ChatGPT/Codex `usage_limit_reached`).
    // Checked before the generic 429 branch below: the outer error text carries
    // "429 Too Many Requests", which would otherwise route it to the transient
    // "wait a moment" rate-limit copy. This condition instead recovers on its
    // own at `resets_at`, so it gets its own code and carries the reset time.
    if is_usage_limit_message(normalized) {
        return UserFacingError::new(codes::PROVIDER_USAGE_LIMIT_REACHED)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone())
            .with_optional_field("resets_at", parse_usage_limit_reset_at(normalized));
    }

    if lower.contains("(429)")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        return UserFacingError::new(codes::PROVIDER_RATE_LIMITED)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone())
            .with_optional_field("retry_after", context.retry_after);
    }

    if lower.contains("(401)") || lower.contains("(403)") {
        return UserFacingError::new(codes::PROVIDER_MISCONFIGURED)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone());
    }

    if lower.contains("api key is required")
        || lower.contains("configure the api key")
        || lower.contains("api key missing")
        || lower.contains("missing api key")
        || lower.contains("invalid api key")
    {
        return UserFacingError::new(codes::PROVIDER_MISCONFIGURED)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone());
    }

    if ["(500)", "(502)", "(503)", "(504)", "(529)"]
        .iter()
        .any(|code| lower.contains(code))
    {
        return UserFacingError::new(codes::PROVIDER_UNAVAILABLE)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone());
    }

    UserFacingError::new(codes::PROCESSING_ERROR)
        .with_optional_field("provider", context.provider.clone())
        .with_optional_field("model_id", context.model_id.clone())
}

fn is_invalid_tool_schema_message(lower: &str) -> bool {
    lower.contains("invalid_function_parameters")
        || lower.contains("invalid function parameters")
        || (lower.contains("invalid json schema") && lower.contains("$.properties"))
        || lower.contains("invalid tool schema")
}

fn extract_schema_path(message: &str) -> Option<String> {
    let path = message.split_once("Found at ")?.1;
    let path = path
        .split(|character: char| character.is_whitespace() || character == '`')
        .next()?
        .trim_end_matches(['.', ',', ';', ':']);
    (path.starts_with('$')
        && path.len() <= 200
        && path.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '$' | '.' | '_' | '-' | '[' | ']')
        }))
    .then(|| path.to_string())
}

pub fn trim_error_chain_prefixes(error_chain: &str) -> &str {
    error_chain
        .trim()
        .trim_start_matches("InputAtom execution failed: ")
        .trim_start_matches("ReasonAtom execution failed: ")
        .trim_start_matches("ActAtom execution failed: ")
}

/// Render the copy for a subscription/plan usage-limit error. The `resets_at`
/// field (unix seconds) is rendered as a UTC fallback; clients localize it into
/// the viewer's timezone from the same raw field. When `auto_continue` is set —
/// added by the emit site only when an auto-continue capability is active — the
/// copy promises automatic resumption; otherwise it stays generic.
fn usage_limit_reached_message(fields: &UserFacingErrorFields) -> String {
    let mut message = String::from("You're out of LLM usage limits.");

    if let Some(resets_at) = number_field(fields, "resets_at")
        && let Some(reset) = chrono::DateTime::from_timestamp(resets_at as i64, 0)
    {
        message.push_str(&format!(
            " Your usage limit resets at {}.",
            reset.format("%H:%M UTC on %b %-d")
        ));
    }

    if bool_field(fields, "auto_continue") {
        message.push_str(" We'll continue work automatically once it resets.");
    }

    message
}

fn budget_exhausted_message(fields: &UserFacingErrorFields) -> String {
    if let (Some(spent), Some(limit), Some(currency)) = (
        number_field(fields, "spent"),
        number_field(fields, "limit"),
        string_field(fields, "currency"),
    ) {
        let comparison = if spent > limit { "exceeded" } else { "reached" };
        return format!(
            "Budget exhausted. {:.2} {} spent {} the {:.2} {} limit. Increase the budget to continue.",
            spent, currency, comparison, limit, currency
        );
    }

    "Budget exhausted. Increase the budget to continue.".to_string()
}

fn budget_paused_message(fields: &UserFacingErrorFields) -> String {
    let spent = number_field(fields, "spent");
    let currency = string_field(fields, "currency");
    let soft_limit = number_field(fields, "soft_limit");

    match (spent, currency, soft_limit) {
        (Some(spent), Some(currency), Some(soft_limit)) => {
            let comparison = if spent > soft_limit {
                "exceeded"
            } else if spent >= soft_limit {
                "reached"
            } else {
                "with"
            };
            if comparison == "with" {
                format!(
                    "Budget paused with {:.2} {} spent. Increase or resume the budget to continue.",
                    spent, currency
                )
            } else {
                format!(
                    "Budget paused. {:.2} {} spent {} the {:.2} {} soft limit. Increase or resume the budget to continue.",
                    spent, currency, comparison, soft_limit, currency
                )
            }
        }
        (Some(spent), Some(currency), None) => format!(
            "Budget paused with {:.2} {} spent. Increase or resume the budget to continue.",
            spent, currency
        ),
        _ => "Budget paused. Increase or resume the budget to continue.".to_string(),
    }
}

fn parse_budget_exhausted_fields(message: &str) -> Option<UserFacingErrorFields> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"^Budget exhausted\. (?P<spent>\d+(?:\.\d+)?) (?P<currency>\S+) spent (?:reached|exceeded) the (?P<limit>\d+(?:\.\d+)?) \S+ limit\.",
        )
        .expect("valid budget exhausted regex")
    });
    let caps = re.captures(message)?;
    Some(
        UserFacingErrorFields::new()
            .with_number("spent", caps.name("spent")?.as_str())
            .with_number("limit", caps.name("limit")?.as_str())
            .with_string("currency", caps.name("currency")?.as_str()),
    )
}

fn parse_budget_paused_fields(message: &str) -> Option<UserFacingErrorFields> {
    static SOFT_LIMIT_RE: OnceLock<Regex> = OnceLock::new();
    static SIMPLE_RE: OnceLock<Regex> = OnceLock::new();

    let soft_limit_re = SOFT_LIMIT_RE.get_or_init(|| {
        Regex::new(
            r"^Budget paused\. (?P<spent>\d+(?:\.\d+)?) (?P<currency>\S+) spent (?:reached|exceeded) the (?P<soft_limit>\d+(?:\.\d+)?) \S+ soft limit\.",
        )
        .expect("valid budget paused regex")
    });
    if let Some(caps) = soft_limit_re.captures(message) {
        return Some(
            UserFacingErrorFields::new()
                .with_number("spent", caps.name("spent")?.as_str())
                .with_number("soft_limit", caps.name("soft_limit")?.as_str())
                .with_string("currency", caps.name("currency")?.as_str()),
        );
    }

    let simple_re = SIMPLE_RE.get_or_init(|| {
        Regex::new(r"^Budget paused with (?P<spent>\d+(?:\.\d+)?) (?P<currency>\S+) spent\.")
            .expect("valid budget paused simple regex")
    });
    let caps = simple_re.captures(message)?;
    Some(
        UserFacingErrorFields::new()
            .with_number("spent", caps.name("spent")?.as_str())
            .with_string("currency", caps.name("currency")?.as_str()),
    )
}

fn string_field<'a>(fields: &'a UserFacingErrorFields, key: &str) -> Option<&'a str> {
    fields.get(key)?.as_str()
}

fn bool_field(fields: &UserFacingErrorFields, key: &str) -> bool {
    fields.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let truncated: String = value.chars().take(max_chars).collect();
    format!("{truncated}\u{2026}")
}

fn number_field(fields: &UserFacingErrorFields, key: &str) -> Option<f64> {
    match fields.get(key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

trait ErrorFieldsExt {
    fn with_string(self, key: &str, value: &str) -> Self;
    fn with_number(self, key: &str, value: &str) -> Self;
}

impl ErrorFieldsExt for UserFacingErrorFields {
    fn with_string(mut self, key: &str, value: &str) -> Self {
        self.insert(key.to_string(), Value::String(value.to_string()));
        self
    }

    fn with_number(mut self, key: &str, value: &str) -> Self {
        if let Ok(number) = value.parse::<f64>()
            && let Some(json_number) = serde_json::Number::from_f64(number)
        {
            self.insert(key.to_string(), Value::Number(json_number));
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_budget_exhausted_parses_fields() {
        let error = classify_runtime_error_message(
            "ReasonAtom execution failed: Budget exhausted. 12.50 usd spent exceeded the 10.00 usd limit. Increase the budget to continue.",
            &UserFacingErrorContext::default(),
        );

        assert_eq!(error.code, codes::BUDGET_EXHAUSTED);
        assert_eq!(number_field(&error.fields, "spent"), Some(12.5));
        assert_eq!(number_field(&error.fields, "limit"), Some(10.0));
        assert_eq!(string_field(&error.fields, "currency"), Some("usd"));
    }

    #[test]
    fn classify_provider_rate_limit_keeps_context() {
        let error = classify_runtime_error_message(
            "OpenAI API error (429): rate limit exceeded",
            &UserFacingErrorContext::default()
                .with_provider("openai")
                .with_model_id("gpt-5")
                .with_retry_after(7),
        );

        assert_eq!(error.code, codes::PROVIDER_RATE_LIMITED);
        assert_eq!(string_field(&error.fields, "provider"), Some("openai"));
        assert_eq!(string_field(&error.fields, "model_id"), Some("gpt-5"));
        assert_eq!(number_field(&error.fields, "retry_after"), Some(7.0));
    }

    #[test]
    fn classifies_openai_tool_schema_rejection_without_exposing_provider_payload() {
        let error = classify_runtime_error_message(
            "OpenAI Responses API error (400 Bad Request): Invalid JSON schema: regex lookaround is not supported. Found at $.properties.email.pattern.",
            &UserFacingErrorContext::default()
                .with_provider("openai")
                .with_model_id("gpt-5.6-terra"),
        );

        assert_eq!(error.code, codes::INVALID_TOOL_SCHEMA);
        assert_eq!(
            error.fields.get("schema_path").and_then(Value::as_str),
            Some("$.properties.email.pattern")
        );
        assert_eq!(
            error.fallback_message(),
            "A connected tool uses an input schema that this model provider does not support. Update the integration or choose a different model provider, then try again."
        );
        assert!(!error.fallback_message().contains("regex lookaround"));
    }

    #[test]
    fn invalid_tool_schema_drops_unsafe_provider_schema_path() {
        let error = classify_runtime_error_message(
            "Invalid JSON schema at $.properties: Found at $.properties.email.pattern?<token>.",
            &UserFacingErrorContext::default(),
        );

        assert_eq!(error.code, codes::INVALID_TOOL_SCHEMA);
        assert!(!error.fields.contains_key("schema_path"));
    }

    #[test]
    fn classify_openai_insufficient_quota_as_provider_quota_exhausted() {
        // OpenAI's exhausted-billing 429 needs operator action (top up the
        // account), not the transient "rate limited, wait a moment" copy and
        // not the "misconfigured" copy used for bad API keys.
        let error = classify_runtime_error_message(
            "ReasonAtom execution failed: OpenAI API error (429): {\"error\":{\"message\":\"You exceeded your current quota, please check your plan and billing details.\",\"type\":\"insufficient_quota\",\"code\":\"insufficient_quota\"}}",
            &UserFacingErrorContext::default()
                .with_provider("openai")
                .with_model_id("gpt-4.1-mini"),
        );

        assert_eq!(error.code, codes::PROVIDER_QUOTA_EXHAUSTED);
        assert_eq!(string_field(&error.fields, "provider"), Some("openai"));
        assert_eq!(
            string_field(&error.fields, "model_id"),
            Some("gpt-4.1-mini")
        );
    }

    #[test]
    fn classify_insufficient_quota_without_status_prefix() {
        // Even if upstream wrapping drops the "(429)" prefix, the explicit
        // quota substring must still route to PROVIDER_QUOTA_EXHAUSTED rather
        // than the canned PROCESSING_ERROR fallback (EVE-472).
        let error = classify_runtime_error_message(
            "LLM error: insufficient_quota: You exceeded your current quota.",
            &UserFacingErrorContext::default(),
        );

        assert_eq!(error.code, codes::PROVIDER_QUOTA_EXHAUSTED);
    }

    #[test]
    fn classify_credit_balance_exhausted_as_provider_quota_exhausted() {
        let error = classify_runtime_error_message(
            "LLM error: credit_balance_exhausted: You have no credits remaining. secret=hidden",
            &UserFacingErrorContext::default(),
        );

        assert_eq!(error.code, codes::PROVIDER_QUOTA_EXHAUSTED);
        assert_eq!(
            error.fallback_message(),
            "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue."
        );
        assert!(!error.fallback_message().contains("secret"));
    }

    #[test]
    fn classify_codex_usage_limit_reached_as_usage_limit() {
        // The Codex/ChatGPT 429 usage-limit body must route to its own code
        // (recovers at `resets_at`) rather than the transient rate-limit copy,
        // and must capture the absolute reset timestamp for clients to localize.
        let error = classify_runtime_error_message(
            "LLM error: Codex API error (429 Too Many Requests): {\"error\":{\"type\":\"usage_limit_reached\",\"message\":\"The usage limit has been reached\",\"plan_type\":\"pro\",\"resets_at\":1783767823,\"eligible_promo\":null,\"resets_in_seconds\":12337}}",
            &UserFacingErrorContext::default()
                .with_provider("openai-codex")
                .with_model_id("gpt-5-codex"),
        );

        assert_eq!(error.code, codes::PROVIDER_USAGE_LIMIT_REACHED);
        assert_eq!(
            string_field(&error.fields, "provider"),
            Some("openai-codex")
        );
        assert_eq!(number_field(&error.fields, "resets_at"), Some(1783767823.0));

        // Base copy is human-readable and names the reset time; without the
        // capability field it makes no automatic-continuation promise.
        let message = error.fallback_message();
        assert!(
            message.starts_with("You're out of LLM usage limits."),
            "unexpected copy: {message}"
        );
        assert!(
            message.contains("resets at"),
            "missing reset time: {message}"
        );
        assert!(
            !message.contains("automatically"),
            "unexpected promise: {message}"
        );
    }

    #[test]
    fn usage_limit_message_without_reset_time_stays_generic() {
        let error = classify_runtime_error_message(
            "Some Provider API error (429): usage limit reached",
            &UserFacingErrorContext::default(),
        );

        assert_eq!(error.code, codes::PROVIDER_USAGE_LIMIT_REACHED);
        assert_eq!(number_field(&error.fields, "resets_at"), None);
        assert_eq!(error.fallback_message(), "You're out of LLM usage limits.");
    }

    #[test]
    fn usage_limit_message_appends_auto_continue_suffix_when_flagged() {
        // The emit site sets `auto_continue` only when an auto-continue
        // capability is active; the copy then promises automatic resumption.
        let error = UserFacingError::new(codes::PROVIDER_USAGE_LIMIT_REACHED)
            .with_field("resets_at", 1783767823)
            .with_field("auto_continue", true);

        let message = error.fallback_message();
        assert!(
            message.contains("resets at"),
            "missing reset time: {message}"
        );
        assert!(
            message.contains("We'll continue work automatically once it resets."),
            "missing auto-continue promise: {message}"
        );
    }

    #[test]
    fn classify_anthropic_low_credit_balance_as_provider_quota_exhausted() {
        let error = classify_runtime_error_message(
            "Anthropic API error (400): {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits.\"}}",
            &UserFacingErrorContext::default().with_provider("anthropic"),
        );

        assert_eq!(error.code, codes::PROVIDER_QUOTA_EXHAUSTED);
    }

    #[test]
    fn disclosure_generic_collapses_code_and_fields() {
        let error = UserFacingError::new(codes::PROVIDER_QUOTA_EXHAUSTED)
            .with_field("provider", "openai")
            .with_field("model_id", "gpt-4.1-mini");

        let disclosed = error.apply_disclosure(ErrorDisclosure::Generic, Some("raw detail"));

        assert_eq!(disclosed.code, codes::PROCESSING_ERROR);
        assert!(disclosed.fields.is_empty());
        assert_eq!(
            disclosed.fallback_message(),
            "I encountered an error while processing your request. Please try again later."
        );
    }

    #[test]
    fn disclosure_standard_is_identity() {
        let error = UserFacingError::new(codes::PROVIDER_RATE_LIMITED).with_field("retry_after", 7);
        let disclosed = error.apply_disclosure(ErrorDisclosure::Standard, Some("raw detail"));
        assert_eq!(disclosed, error);
    }

    #[test]
    fn disclosure_detailed_attaches_detail_without_rendering_it() {
        let error = UserFacingError::new(codes::PROVIDER_QUOTA_EXHAUSTED);
        let disclosed = error.apply_disclosure(
            ErrorDisclosure::Detailed,
            Some("OpenAI API error (429): insufficient_quota Authorization: Bearer sk-secret"),
        );

        assert_eq!(disclosed.code, codes::PROVIDER_QUOTA_EXHAUSTED);
        assert_eq!(
            string_field(&disclosed.fields, "detail"),
            Some("OpenAI API error (429): insufficient_quota Authorization: Bearer sk-secret")
        );
        let message = disclosed.fallback_message();
        assert!(message.contains("out of credits or quota"));
        assert!(!message.contains("insufficient_quota"));
        assert!(!message.contains("sk-secret"));
    }

    #[test]
    fn disclosure_detailed_truncates_long_detail() {
        let error = UserFacingError::new(codes::PROCESSING_ERROR);
        let long_detail = "x".repeat(5000);
        let disclosed = error.apply_disclosure(ErrorDisclosure::Detailed, Some(&long_detail));
        let detail = string_field(&disclosed.fields, "detail").unwrap();
        assert!(detail.chars().count() <= 1001); // 1000 + ellipsis
    }

    #[test]
    fn disclosure_parse_and_ordering() {
        assert_eq!(
            ErrorDisclosure::parse("Generic"),
            Some(ErrorDisclosure::Generic)
        );
        assert_eq!(
            ErrorDisclosure::parse("detailed"),
            Some(ErrorDisclosure::Detailed)
        );
        assert_eq!(ErrorDisclosure::parse("nope"), None);
        assert!(ErrorDisclosure::Generic < ErrorDisclosure::Standard);
        assert!(ErrorDisclosure::Standard < ErrorDisclosure::Detailed);
        assert_eq!(ErrorDisclosure::default(), ErrorDisclosure::Standard);
    }

    #[test]
    fn classify_missing_api_key_as_provider_misconfigured() {
        let error = classify_runtime_error_message(
            "LLM error: API key is required. Configure the API key in provider settings.",
            &UserFacingErrorContext::default().with_provider("openai"),
        );

        assert_eq!(error.code, codes::PROVIDER_MISCONFIGURED);
        assert_eq!(string_field(&error.fields, "provider"), Some("openai"));
    }

    #[test]
    fn classify_missing_model_as_model_not_configured() {
        let error = classify_runtime_error_message(
            "ReasonAtom execution failed: Model not configured",
            &UserFacingErrorContext::default(),
        );

        assert_eq!(error.code, codes::MODEL_NOT_CONFIGURED);
        assert!(error.fallback_message().contains("Choose a model"));
    }

    #[test]
    fn fallback_message_reuses_budget_fields() {
        let error = UserFacingError::new(codes::BUDGET_PAUSED)
            .with_field("spent", 5.0)
            .with_field("soft_limit", 5.0)
            .with_field("currency", "tokens");

        assert_eq!(
            error.fallback_message(),
            "Budget paused. 5.00 tokens spent reached the 5.00 tokens soft limit. Increase or resume the budget to continue."
        );
    }
}
