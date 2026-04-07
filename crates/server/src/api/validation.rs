// Input validation for agent APIs
//
// Last-resort validation limits to guard Everruns from abuse.
// These are hard limits, not configurable. Values chosen to allow legitimate
// use while preventing resource exhaustion attacks.

use super::common::ErrorResponse;
use axum::Json;
use axum::http::StatusCode;
use everruns_core::localization::is_allowed_locale;
use everruns_core::{InitialFile, SessionFile};
use std::collections::HashSet;

// =============================================================================
// Input Size Limits
// =============================================================================

/// Maximum size for agent name field.
/// 2 KB should accommodate any reasonable agent name.
pub const MAX_AGENT_NAME_BYTES: usize = 2 * 1024; // 2 KB

/// Maximum size for agent description field.
/// 10 KB allows for detailed descriptions with formatting.
pub const MAX_AGENT_DESCRIPTION_BYTES: usize = 10 * 1024; // 10 KB

/// Maximum size for agent system prompt.
/// 1 MB allows for very detailed prompts including embedded context.
pub const MAX_AGENT_SYSTEM_PROMPT_BYTES: usize = 1024 * 1024; // 1 MB

/// Maximum number of capabilities that can be assigned to an agent.
/// 250 is generous for any practical use case.
pub const MAX_AGENT_CAPABILITIES: usize = 250;

/// Maximum size for agent import file.
/// 3 MB accommodates large system prompts with metadata.
pub const MAX_AGENT_IMPORT_FILE_BYTES: usize = 3 * 1024 * 1024; // 3 MB
/// Maximum size for locale tags.
pub const MAX_LOCALE_BYTES: usize = 64;

/// Maximum length for harness name (addressable slug).
pub const MAX_HARNESS_NAME_LEN: usize = 64;

/// Maximum number of starter files on an agent or harness.
pub const MAX_INITIAL_FILES: usize = 100;

/// Maximum total decoded bytes across all starter files.
pub const MAX_INITIAL_FILES_TOTAL_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Generic validation error message returned to clients.
/// Intentionally vague to avoid leaking which field exceeded limits.
pub const VALIDATION_ERROR_MESSAGE: &str = "Input exceeds allowed limits";

// =============================================================================
// Validation Functions
// =============================================================================

/// Validation error - returns generic message to avoid leaking details
#[derive(Debug)]
pub struct ValidationError;

impl From<ValidationError> for StatusCode {
    fn from(_: ValidationError) -> Self {
        StatusCode::BAD_REQUEST
    }
}

impl From<ValidationError> for (StatusCode, String) {
    fn from(_: ValidationError) -> Self {
        (
            StatusCode::BAD_REQUEST,
            VALIDATION_ERROR_MESSAGE.to_string(),
        )
    }
}

impl From<ValidationError> for (StatusCode, Json<ErrorResponse>) {
    fn from(_: ValidationError) -> Self {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(VALIDATION_ERROR_MESSAGE)),
        )
    }
}

/// Validate agent name size
pub fn validate_agent_name(name: &str) -> Result<(), ValidationError> {
    if name.len() > MAX_AGENT_NAME_BYTES {
        tracing::warn!(
            "Agent name exceeds limit: {} bytes (max: {})",
            name.len(),
            MAX_AGENT_NAME_BYTES
        );
        return Err(ValidationError);
    }
    Ok(())
}

/// Names that cannot be used for user-created harnesses because they have
/// special meaning (virtual aliases, future system harnesses).
pub const RESERVED_HARNESS_NAMES: &[&str] = &["default"];

/// Validate harness name/alias is a valid slug: `[a-z0-9]([a-z0-9-]*[a-z0-9])?`.
/// Max 64 chars, no consecutive hyphens, no leading/trailing hyphens.
/// Accepts reserved names (virtual aliases like "default"). Use this for
/// endpoints that resolve names, e.g. session creation with `harness_name`.
pub fn validate_harness_name(name: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    validate_harness_name_inner(name)
}

/// Validate harness name for create/update — same slug rules as
/// [`validate_harness_name`] but additionally rejects reserved names.
pub fn validate_harness_name_strict(name: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    validate_harness_name_inner(name)?;
    if RESERVED_HARNESS_NAMES.contains(&name) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(format!(
                "Harness name '{name}' is reserved and cannot be used"
            ))),
        ));
    }
    Ok(())
}

fn validate_harness_name_inner(name: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("Harness name must not be empty")),
        ));
    }
    if name.len() > MAX_HARNESS_NAME_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(format!(
                "Harness name must be at most {} characters",
                MAX_HARNESS_NAME_LEN
            ))),
        ));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Harness name must contain only lowercase letters, digits, and hyphens",
            )),
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Harness name must not start or end with a hyphen",
            )),
        ));
    }
    if name.contains("--") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Harness name must not contain consecutive hyphens",
            )),
        ));
    }
    Ok(())
}

/// Validate agent description size
pub fn validate_agent_description(description: Option<&str>) -> Result<(), ValidationError> {
    if let Some(desc) = description
        && desc.len() > MAX_AGENT_DESCRIPTION_BYTES
    {
        tracing::warn!(
            "Agent description exceeds limit: {} bytes (max: {})",
            desc.len(),
            MAX_AGENT_DESCRIPTION_BYTES
        );
        return Err(ValidationError);
    }
    Ok(())
}

/// Validate agent system prompt size
pub fn validate_agent_system_prompt(system_prompt: &str) -> Result<(), ValidationError> {
    if system_prompt.len() > MAX_AGENT_SYSTEM_PROMPT_BYTES {
        tracing::warn!(
            "Agent system prompt exceeds limit: {} bytes (max: {})",
            system_prompt.len(),
            MAX_AGENT_SYSTEM_PROMPT_BYTES
        );
        return Err(ValidationError);
    }
    Ok(())
}

/// Validate capabilities count
pub fn validate_agent_capabilities_count(count: usize) -> Result<(), ValidationError> {
    if count > MAX_AGENT_CAPABILITIES {
        tracing::warn!(
            "Agent capabilities count exceeds limit: {} (max: {})",
            count,
            MAX_AGENT_CAPABILITIES
        );
        return Err(ValidationError);
    }
    Ok(())
}

/// Validate import file size
pub fn validate_import_file_size(size: usize) -> Result<(), ValidationError> {
    if size > MAX_AGENT_IMPORT_FILE_BYTES {
        tracing::warn!(
            "Agent import file exceeds limit: {} bytes (max: {})",
            size,
            MAX_AGENT_IMPORT_FILE_BYTES
        );
        return Err(ValidationError);
    }
    Ok(())
}

/// Validate and normalize a locale tag.
///
/// Normalizes `_` separators to `-`, then checks against the centralized
/// allowlist in `everruns_core::localization`.
pub fn normalize_locale(locale: Option<String>) -> Result<Option<String>, ValidationError> {
    let Some(locale) = locale else {
        return Ok(None);
    };

    let normalized = locale.trim().replace('_', "-");
    if normalized.is_empty() || normalized.len() > MAX_LOCALE_BYTES {
        tracing::warn!("Locale exceeds limit or is empty: {:?}", locale);
        return Err(ValidationError);
    }

    if !is_allowed_locale(&normalized) {
        tracing::warn!("Locale not in allowed set: {:?}", normalized);
        return Err(ValidationError);
    }

    Ok(Some(normalized))
}

/// Validate and normalize a locale override inside message controls.
pub fn normalize_controls_locale(
    controls: Option<everruns_core::Controls>,
) -> Result<Option<everruns_core::Controls>, ValidationError> {
    let Some(mut controls) = controls else {
        return Ok(None);
    };
    controls.locale = normalize_locale(controls.locale)?;
    Ok(Some(controls))
}

/// Validate starter files collection and per-file shape.
pub fn validate_initial_files(initial_files: &[InitialFile]) -> Result<(), ValidationError> {
    if initial_files.len() > MAX_INITIAL_FILES {
        tracing::warn!(
            "Initial files count exceeds limit: {} (max: {})",
            initial_files.len(),
            MAX_INITIAL_FILES
        );
        return Err(ValidationError);
    }

    let mut total_bytes = 0usize;
    let mut seen_paths = HashSet::new();

    for file in initial_files {
        let normalized = normalize_initial_file_path(&file.path).ok_or(ValidationError)?;

        if !seen_paths.insert(normalized) {
            tracing::warn!("Duplicate initial file path");
            return Err(ValidationError);
        }

        if file.encoding != "text" && file.encoding != "base64" {
            tracing::warn!("Invalid initial file encoding");
            return Err(ValidationError);
        }

        let decoded = SessionFile::decode_content(&file.content, &file.encoding)
            .map_err(|_| ValidationError)?;
        total_bytes = total_bytes.saturating_add(decoded.len());
        if total_bytes > MAX_INITIAL_FILES_TOTAL_BYTES {
            tracing::warn!(
                "Initial files total exceeds limit: {} bytes (max: {})",
                total_bytes,
                MAX_INITIAL_FILES_TOTAL_BYTES
            );
            return Err(ValidationError);
        }
    }

    Ok(())
}

/// Validate all fields for CreateAgentRequest
pub fn validate_create_agent_input(
    name: &str,
    description: Option<&str>,
    system_prompt: &str,
    capabilities_count: usize,
    initial_files: &[InitialFile],
) -> Result<(), ValidationError> {
    validate_agent_name(name)?;
    validate_agent_description(description)?;
    validate_agent_system_prompt(system_prompt)?;
    validate_agent_capabilities_count(capabilities_count)?;
    validate_initial_files(initial_files)?;
    Ok(())
}

/// Validate all provided fields for UpdateAgentRequest
pub fn validate_update_agent_input(
    name: Option<&str>,
    description: Option<&str>,
    system_prompt: Option<&str>,
    capabilities_count: Option<usize>,
    initial_files: Option<&[InitialFile]>,
) -> Result<(), ValidationError> {
    if let Some(name) = name {
        validate_agent_name(name)?;
    }
    // For update, description is Option<Option<String>> effectively
    // but we receive Option<String> - validate if present
    if let Some(desc) = description
        && desc.len() > MAX_AGENT_DESCRIPTION_BYTES
    {
        tracing::warn!(
            "Agent description exceeds limit: {} bytes (max: {})",
            desc.len(),
            MAX_AGENT_DESCRIPTION_BYTES
        );
        return Err(ValidationError);
    }
    if let Some(prompt) = system_prompt {
        validate_agent_system_prompt(prompt)?;
    }
    if let Some(count) = capabilities_count {
        validate_agent_capabilities_count(count)?;
    }
    if let Some(initial_files) = initial_files {
        validate_initial_files(initial_files)?;
    }
    Ok(())
}

fn normalize_initial_file_path(path: &str) -> Option<String> {
    let normalized = if path == "/workspace" {
        return None;
    } else if let Some(stripped) = path.strip_prefix("/workspace/") {
        format!("/{}", stripped.trim_start_matches('/'))
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    if normalized == "/"
        || normalized.contains('\0')
        || normalized.split('/').any(|segment| segment == "..")
        || normalized.contains("//")
    {
        return None;
    }

    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_agent_name() {
        assert!(validate_agent_name("My Agent").is_ok());
        assert!(validate_agent_name(&"x".repeat(MAX_AGENT_NAME_BYTES)).is_ok());
    }

    #[test]
    fn test_invalid_agent_name() {
        assert!(validate_agent_name(&"x".repeat(MAX_AGENT_NAME_BYTES + 1)).is_err());
    }

    #[test]
    fn test_valid_description() {
        assert!(validate_agent_description(None).is_ok());
        assert!(validate_agent_description(Some("Short description")).is_ok());
        assert!(validate_agent_description(Some(&"x".repeat(MAX_AGENT_DESCRIPTION_BYTES))).is_ok());
    }

    #[test]
    fn test_invalid_description() {
        assert!(
            validate_agent_description(Some(&"x".repeat(MAX_AGENT_DESCRIPTION_BYTES + 1))).is_err()
        );
    }

    #[test]
    fn test_valid_system_prompt() {
        assert!(validate_agent_system_prompt("You are helpful.").is_ok());
        assert!(validate_agent_system_prompt(&"x".repeat(MAX_AGENT_SYSTEM_PROMPT_BYTES)).is_ok());
    }

    #[test]
    fn test_invalid_system_prompt() {
        assert!(
            validate_agent_system_prompt(&"x".repeat(MAX_AGENT_SYSTEM_PROMPT_BYTES + 1)).is_err()
        );
    }

    #[test]
    fn test_normalize_locale_accepts_bcp47_style_tags() {
        assert_eq!(
            normalize_locale(Some("uk-UA".to_string())).unwrap(),
            Some("uk-UA".to_string())
        );
        assert_eq!(
            normalize_locale(Some("uk_UA".to_string())).unwrap(),
            Some("uk-UA".to_string())
        );
    }

    #[test]
    fn test_normalize_locale_rejects_invalid_tags() {
        assert!(normalize_locale(Some("".to_string())).is_err());
        assert!(normalize_locale(Some("uk UA".to_string())).is_err());
        assert!(normalize_locale(Some("-uk".to_string())).is_err());
    }

    #[test]
    fn test_normalize_controls_locale_updates_message_controls() {
        let controls = everruns_core::Controls {
            model_id: None,
            locale: Some("uk_UA".to_string()),
            reasoning: None,
            hints: None,
        };

        let normalized = normalize_controls_locale(Some(controls)).unwrap().unwrap();
        assert_eq!(normalized.locale.as_deref(), Some("uk-UA"));
    }

    #[test]
    fn test_valid_capabilities_count() {
        assert!(validate_agent_capabilities_count(0).is_ok());
        assert!(validate_agent_capabilities_count(MAX_AGENT_CAPABILITIES).is_ok());
    }

    #[test]
    fn test_invalid_capabilities_count() {
        assert!(validate_agent_capabilities_count(MAX_AGENT_CAPABILITIES + 1).is_err());
    }

    #[test]
    fn test_valid_harness_name() {
        assert!(validate_harness_name("my-harness").is_ok());
        assert!(validate_harness_name("harness1").is_ok());
        assert!(validate_harness_name("a").is_ok());
    }

    #[test]
    fn test_harness_name_accepts_reserved_aliases() {
        // validate_harness_name (non-strict) allows reserved names so that
        // endpoints accepting aliases (e.g. session creation) can pass them.
        assert!(validate_harness_name("default").is_ok());
    }

    #[test]
    fn test_harness_name_strict_rejects_reserved() {
        let err = validate_harness_name_strict("default").unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        // Non-reserved names still pass strict validation
        assert!(validate_harness_name_strict("my-harness").is_ok());
    }

    #[test]
    fn test_invalid_harness_name() {
        assert!(validate_harness_name("").is_err());
        assert!(validate_harness_name("-leading").is_err());
        assert!(validate_harness_name("trailing-").is_err());
        assert!(validate_harness_name("bad--double").is_err());
        assert!(validate_harness_name("UPPERCASE").is_err());
    }

    #[test]
    fn test_valid_import_file_size() {
        assert!(validate_import_file_size(0).is_ok());
        assert!(validate_import_file_size(MAX_AGENT_IMPORT_FILE_BYTES).is_ok());
    }

    #[test]
    fn test_invalid_import_file_size() {
        assert!(validate_import_file_size(MAX_AGENT_IMPORT_FILE_BYTES + 1).is_err());
    }
}
