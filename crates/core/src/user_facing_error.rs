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
    pub const REQUEST_TOO_LARGE: &str = "request_too_large";
    pub const PROVIDER_RATE_LIMITED: &str = "provider_rate_limited";
    pub const PROVIDER_MISCONFIGURED: &str = "provider_misconfigured";
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    pub const PROCESSING_ERROR: &str = "processing_error";
    pub const DEPENDENCY_UNAVAILABLE: &str = "dependency_unavailable";
    pub const MAX_ITERATIONS: &str = "max_iterations";
    pub const SOFT_LIMIT_REACHED: &str = "soft_limit_reached";
    /// A `user_prompt_submit` hook rejected the inbound user message.
    pub const BLOCKED_BY_HOOK: &str = "blocked_by_hook";
}

pub type UserFacingErrorFields = BTreeMap<String, Value>;

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

    pub fn fallback_message(&self) -> String {
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
            codes::REQUEST_TOO_LARGE => {
                "The conversation has become too long for the model to process. Please start a new session or reduce the context size.".to_string()
            }
            codes::PROVIDER_RATE_LIMITED => {
                "Rate limited by the AI provider. Please wait a moment.".to_string()
            }
            codes::PROVIDER_MISCONFIGURED => {
                "There is a misconfiguration with the AI provider. Please contact support."
                    .to_string()
            }
            codes::PROVIDER_UNAVAILABLE => {
                "The AI provider is experiencing issues. Please try again shortly.".to_string()
            }
            codes::DEPENDENCY_UNAVAILABLE => {
                "Execution stopped because a required dependency is unavailable.".to_string()
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

    if normalized.starts_with("Request too large:")
        || lower.contains("context length")
        || lower.contains("maximum context length")
    {
        return UserFacingError::new(codes::REQUEST_TOO_LARGE)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone());
    }

    // OpenAI surfaces an exhausted-billing state as HTTP 429 + body
    // `{"error":{"type":"insufficient_quota",...}}`. The "(429)" prefix would
    // otherwise route it to PROVIDER_RATE_LIMITED ("wait a moment"), but the
    // condition is non-transient and needs operator action (top up the
    // account or rotate the key), so classify it as PROVIDER_MISCONFIGURED.
    if lower.contains("insufficient_quota")
        || lower.contains("insufficient quota")
        || lower.contains("exceeded your current quota")
    {
        return UserFacingError::new(codes::PROVIDER_MISCONFIGURED)
            .with_optional_field("provider", context.provider.clone())
            .with_optional_field("model_id", context.model_id.clone());
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

pub fn trim_error_chain_prefixes(error_chain: &str) -> &str {
    error_chain
        .trim()
        .trim_start_matches("InputAtom execution failed: ")
        .trim_start_matches("ReasonAtom execution failed: ")
        .trim_start_matches("ActAtom execution failed: ")
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
    fn classify_openai_insufficient_quota_as_provider_misconfigured() {
        // OpenAI's exhausted-billing 429 needs operator action (top up or
        // rotate key), not the transient "rate limited, wait a moment" copy.
        let error = classify_runtime_error_message(
            "ReasonAtom execution failed: OpenAI API error (429): {\"error\":{\"message\":\"You exceeded your current quota, please check your plan and billing details.\",\"type\":\"insufficient_quota\",\"code\":\"insufficient_quota\"}}",
            &UserFacingErrorContext::default()
                .with_provider("openai")
                .with_model_id("gpt-4.1-mini"),
        );

        assert_eq!(error.code, codes::PROVIDER_MISCONFIGURED);
        assert_eq!(string_field(&error.fields, "provider"), Some("openai"));
        assert_eq!(
            string_field(&error.fields, "model_id"),
            Some("gpt-4.1-mini")
        );
    }

    #[test]
    fn classify_insufficient_quota_without_status_prefix() {
        // Even if upstream wrapping drops the "(429)" prefix, the explicit
        // quota substring must still route to PROVIDER_MISCONFIGURED rather
        // than the canned PROCESSING_ERROR fallback (EVE-472).
        let error = classify_runtime_error_message(
            "LLM error: insufficient_quota: You exceeded your current quota.",
            &UserFacingErrorContext::default(),
        );

        assert_eq!(error.code, codes::PROVIDER_MISCONFIGURED);
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
