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
        } else {
            // Reusing a metadata map must not retain fields from an older,
            // more detailed error after disclosure has removed them.
            metadata.remove("error_fields");
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
    use serde_json::json;

    fn wire(error: &UserFacingError) -> Value {
        serde_json::to_value(error).unwrap()
    }
    fn context() -> UserFacingErrorContext {
        UserFacingErrorContext::default()
            .with_provider("provider")
            .with_model_id("model")
            .with_retry_after(7)
    }

    #[test]
    fn quota_classification_preserves_context_without_raw_payload_or_retry_delay() {
        for message in [
            "ReasonAtom execution failed: OpenAI API error (429): {\"error\":{\"type\":\"insufficient_quota\",\"message\":\"You exceeded your current quota\"}}",
            "LLM error: insufficient_quota: You exceeded your current quota.",
            "credit_balance_exhausted: secret=hidden",
            "Anthropic API error (400): Your credit balance is too low to access the Anthropic API.",
            "INSUFFICIENT QUOTA",
        ] {
            let error = classify_runtime_error_message(message, &context());
            assert_eq!(
                wire(&error),
                json!({"code":"provider_quota_exhausted","fields":{"provider":"provider","model_id":"model"}})
            );
            assert_eq!(
                error.fallback_message(),
                "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue."
            );
            assert_eq!(
                wire(&classify_runtime_error_message(
                    message,
                    &UserFacingErrorContext::default()
                )),
                json!({"code":"provider_quota_exhausted"})
            );
        }
    }

    #[test]
    fn ordinary_classification_has_exact_code_and_allowed_context_fields() {
        for (message, expected) in [
            (
                "OpenAI API error (429): rate limit exceeded",
                json!({"code":"provider_rate_limited","fields":{"provider":"provider","model_id":"model","retry_after":7}}),
            ),
            (
                "LLM error: API key is required. Configure the API key in provider settings.",
                json!({"code":"provider_misconfigured","fields":{"provider":"provider","model_id":"model"}}),
            ),
            (
                "ReasonAtom execution failed: Model not configured",
                json!({"code":"model_not_configured"}),
            ),
            (
                "ActAtom execution failed: Model not available: retired-model",
                json!({"code":"model_unavailable","fields":{"model_id":"retired-model"}}),
            ),
            (
                "Request too large: context length",
                json!({"code":"request_too_large","fields":{"provider":"provider","model_id":"model"}}),
            ),
            (
                "provider error (503)",
                json!({"code":"provider_unavailable","fields":{"provider":"provider","model_id":"model"}}),
            ),
            (
                "unknown raw error secret=hidden",
                json!({"code":"processing_error","fields":{"provider":"provider","model_id":"model"}}),
            ),
        ] {
            assert_eq!(
                wire(&classify_runtime_error_message(message, &context())),
                expected,
                "{message}"
            );
        }
        assert_eq!(
            UserFacingError::new("model_not_configured").fallback_message(),
            "No model is configured for this chat. Choose a model or configure a default model, then try again."
        );
    }

    #[test]
    fn budget_fields_drive_exact_exhausted_and_paused_copy() {
        let error = classify_runtime_error_message(
            "ReasonAtom execution failed: Budget exhausted. 12.50 usd spent exceeded the 10.00 usd limit. Increase the budget to continue.",
            &context(),
        );
        assert_eq!(
            wire(&error),
            json!({"code":"budget_exhausted","fields":{"spent":12.5,"limit":10.0,"currency":"usd"}})
        );
        assert_eq!(
            error.fallback_message(),
            "Budget exhausted. 12.50 usd spent exceeded the 10.00 usd limit. Increase the budget to continue."
        );
        for (spent, expected) in [
            (
                4.0,
                "Budget paused with 4.00 tokens spent. Increase or resume the budget to continue.",
            ),
            (
                5.0,
                "Budget paused. 5.00 tokens spent reached the 5.00 tokens soft limit. Increase or resume the budget to continue.",
            ),
            (
                6.0,
                "Budget paused. 6.00 tokens spent exceeded the 5.00 tokens soft limit. Increase or resume the budget to continue.",
            ),
        ] {
            let error = UserFacingError::new("budget_paused")
                .with_field("spent", spent)
                .with_field("soft_limit", 5.0)
                .with_field("currency", "tokens");
            assert_eq!(error.fallback_message(), expected);
        }
        assert_eq!(
            UserFacingError::new("budget_paused").fallback_message(),
            "Budget paused. Increase or resume the budget to continue."
        );
    }

    #[test]
    fn schema_rejections_only_expose_safe_bounded_paths() {
        let path200 = format!("$.{}", "a".repeat(198));
        let path201 = format!("$.{}", "a".repeat(199));
        for (path, expected_path) in [
            (
                "$.properties.email.pattern",
                Some("$.properties.email.pattern"),
            ),
            ("$.properties.email.pattern?<token>", None),
            (path200.as_str(), Some(path200.as_str())),
            (path201.as_str(), None),
        ] {
            let error = classify_runtime_error_message(
                &format!(
                    "Invalid JSON schema at $.properties: regex lookaround is unsupported. Found at {path}."
                ),
                &context(),
            );
            let mut fields = json!({"provider":"provider","model_id":"model"});
            if let Some(path) = expected_path {
                fields["schema_path"] = json!(path);
            }
            assert_eq!(
                wire(&error),
                json!({"code":"invalid_tool_schema","fields":fields})
            );
            assert_eq!(
                error.fallback_message(),
                "A connected tool uses an input schema that this model provider does not support. Update the integration or choose a different model provider, then try again."
            );
        }
    }

    #[test]
    fn usage_limits_have_exact_reset_copy_and_explicit_auto_continue_policy() {
        let error = classify_runtime_error_message(
            "Codex API error (429 Too Many Requests): {\"error\":{\"type\":\"usage_limit_reached\",\"resets_at\":1783767823,\"resets_in_seconds\":12337}}",
            &context(),
        );
        assert_eq!(
            wire(&error),
            json!({"code":"provider_usage_limit_reached","fields":{"provider":"provider","model_id":"model","resets_at":1783767823}})
        );
        assert_eq!(
            error.fallback_message(),
            "You're out of LLM usage limits. Your usage limit resets at 11:03 UTC on Jul 11."
        );
        assert_eq!(
            error
                .clone()
                .with_field("auto_continue", true)
                .fallback_message(),
            "You're out of LLM usage limits. Your usage limit resets at 11:03 UTC on Jul 11. We'll continue work automatically once it resets."
        );
        assert_eq!(
            error
                .clone()
                .with_field("auto_continue", false)
                .fallback_message(),
            error.fallback_message()
        );
        let no_reset = classify_runtime_error_message(
            "Some Provider API error (429): usage limit reached",
            &UserFacingErrorContext::default(),
        );
        assert_eq!(
            wire(&no_reset),
            json!({"code":"provider_usage_limit_reached"})
        );
        assert_eq!(
            no_reset.fallback_message(),
            "You're out of LLM usage limits."
        );
    }

    #[test]
    fn disclosure_modes_preserve_only_their_allowed_fields() {
        let error = UserFacingError::new("provider_quota_exhausted")
            .with_field("provider", "openai")
            .with_field("model_id", "model");
        let detail = " Authorization: Bearer synthetic-secret ";
        let generic = error.apply_disclosure(ErrorDisclosure::Generic, Some(detail));
        assert_eq!(wire(&generic), json!({"code":"processing_error"}));
        assert_eq!(
            generic.fallback_message(),
            "I encountered an error while processing your request. Please try again later."
        );
        assert_eq!(
            error.apply_disclosure(ErrorDisclosure::Standard, Some(detail)),
            error
        );
        let detailed = error.apply_disclosure(ErrorDisclosure::Detailed, Some(detail));
        assert_eq!(
            wire(&detailed),
            json!({"code":"provider_quota_exhausted","fields":{"provider":"openai","model_id":"model","detail":"Authorization: Bearer synthetic-secret"}})
        );
        assert_eq!(
            detailed.fallback_message(),
            "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue."
        );
        for empty in [None, Some(""), Some(" \n\t")] {
            assert_eq!(
                error.apply_disclosure(ErrorDisclosure::Detailed, empty),
                error
            );
        }
    }

    #[test]
    fn detailed_disclosure_has_literal_unicode_character_boundary() {
        let error = UserFacingError::new("processing_error");
        for length in [999, 1000, 1001] {
            let input = "🦀".repeat(length);
            let expected = if length <= 1000 {
                input.clone()
            } else {
                format!("{}…", "🦀".repeat(1000))
            };
            assert_eq!(
                wire(&error.apply_disclosure(ErrorDisclosure::Detailed, Some(&input))),
                json!({"code":"processing_error","fields":{"detail":expected}})
            );
        }
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
    fn applying_error_replaces_owned_metadata_and_clears_previous_detail() {
        let mut metadata = HashMap::from([
            ("other".into(), json!("preserve")),
            (
                "error_fields".into(),
                json!({"detail":"old-private-detail"}),
            ),
            ("error_code".into(), json!("old-code")),
        ]);
        let error = UserFacingError::new("provider_rate_limited").with_field("retry_after", 7);
        error.apply_to_message_metadata(&mut metadata);
        assert_eq!(
            metadata,
            HashMap::from([
                ("other".into(), json!("preserve")),
                ("error_code".into(), json!("provider_rate_limited")),
                ("error_fields".into(), json!({"retry_after":7}))
            ])
        );
        let generic = error.apply_disclosure(ErrorDisclosure::Generic, None);
        generic.apply_to_message_metadata(&mut metadata);
        assert_eq!(
            metadata,
            HashMap::from([
                ("other".into(), json!("preserve")),
                ("error_code".into(), json!("processing_error"))
            ])
        );
        let mut code = Some("old-code".into());
        let mut fields = Some(BTreeMap::from([(
            "detail".into(),
            json!("old-private-detail"),
        )]));
        generic.apply_to_event_fields(&mut code, &mut fields);
        assert_eq!((code, fields), (Some("processing_error".into()), None));
        UserFacingError::apply_disclosure_to_message_metadata(
            &mut metadata,
            ErrorDisclosure::Generic,
            "provider_rate_limited",
        );
        assert_eq!(
            metadata,
            HashMap::from([
                ("other".into(), json!("preserve")),
                ("error_code".into(), json!("processing_error")),
                ("error_disclosure".into(), json!("generic")),
                ("source_error_code".into(), json!("provider_rate_limited"))
            ])
        );
    }
}
