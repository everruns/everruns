// Input validation for agent APIs
//
// Last-resort validation limits to guard Everruns from abuse.
// These are hard limits, not configurable. Values chosen to allow legitimate
// use while preventing resource exhaustion attacks.

use super::common::ErrorResponse;
use axum::http::StatusCode;
use axum::Json;

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

/// Generic validation error message returned to clients.
/// Intentionally vague to avoid leaking which field exceeded limits.
pub const VALIDATION_ERROR_MESSAGE: &str = "Input exceeds allowed limits";

// =============================================================================
// Validation Functions
// =============================================================================

/// Validation error - returns generic message to avoid leaking details
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

/// Validate agent description size
pub fn validate_agent_description(description: Option<&str>) -> Result<(), ValidationError> {
    if let Some(desc) = description {
        if desc.len() > MAX_AGENT_DESCRIPTION_BYTES {
            tracing::warn!(
                "Agent description exceeds limit: {} bytes (max: {})",
                desc.len(),
                MAX_AGENT_DESCRIPTION_BYTES
            );
            return Err(ValidationError);
        }
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

/// Validate all fields for CreateAgentRequest
pub fn validate_create_agent_input(
    name: &str,
    description: Option<&str>,
    system_prompt: &str,
    capabilities_count: usize,
) -> Result<(), ValidationError> {
    validate_agent_name(name)?;
    validate_agent_description(description)?;
    validate_agent_system_prompt(system_prompt)?;
    validate_agent_capabilities_count(capabilities_count)?;
    Ok(())
}

/// Validate all provided fields for UpdateAgentRequest
pub fn validate_update_agent_input(
    name: Option<&str>,
    description: Option<&str>,
    system_prompt: Option<&str>,
    capabilities_count: Option<usize>,
) -> Result<(), ValidationError> {
    if let Some(name) = name {
        validate_agent_name(name)?;
    }
    // For update, description is Option<Option<String>> effectively
    // but we receive Option<String> - validate if present
    if let Some(desc) = description {
        if desc.len() > MAX_AGENT_DESCRIPTION_BYTES {
            tracing::warn!(
                "Agent description exceeds limit: {} bytes (max: {})",
                desc.len(),
                MAX_AGENT_DESCRIPTION_BYTES
            );
            return Err(ValidationError);
        }
    }
    if let Some(prompt) = system_prompt {
        validate_agent_system_prompt(prompt)?;
    }
    if let Some(count) = capabilities_count {
        validate_agent_capabilities_count(count)?;
    }
    Ok(())
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
    fn test_valid_capabilities_count() {
        assert!(validate_agent_capabilities_count(0).is_ok());
        assert!(validate_agent_capabilities_count(MAX_AGENT_CAPABILITIES).is_ok());
    }

    #[test]
    fn test_invalid_capabilities_count() {
        assert!(validate_agent_capabilities_count(MAX_AGENT_CAPABILITIES + 1).is_err());
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
