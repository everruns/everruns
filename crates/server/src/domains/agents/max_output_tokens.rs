//! Agent `max_output_tokens` validation and storage conversion.
//!
//! The ceiling is sent to the provider as `max_tokens` on every call. Some
//! providers (OpenRouter) otherwise reserve the model's full output limit
//! against the account balance and reject the call outright, so a low ceiling
//! is how an agent fits a small balance. Storage is `INTEGER`; values outside
//! `1..=i32::MAX` are rejected, never cast.

use crate::domains::common::CommandError;
use crate::storage::UpdateField;

/// Reject zero and values the `INTEGER` column cannot hold.
pub fn validate(value: Option<u32>) -> Result<(), CommandError> {
    match value {
        Some(tokens) if tokens == 0 || i32::try_from(tokens).is_err() => {
            Err(CommandError::bad_request(format!(
                "max_output_tokens must be between 1 and {}",
                i32::MAX
            )))
        }
        _ => Ok(()),
    }
}

/// Convert a validated value to its column value.
pub fn to_db(value: Option<u32>) -> Option<i32> {
    value.and_then(|tokens| i32::try_from(tokens).ok())
}

/// Validate a PATCH value; clearing is always allowed.
pub fn validate_update(value: &UpdateField<u32>) -> Result<(), CommandError> {
    match value {
        UpdateField::Set(tokens) => validate(Some(*tokens)),
        _ => Ok(()),
    }
}

/// Convert a validated PATCH value to the repository's set/clear shape.
pub fn update_to_db(value: UpdateField<u32>) -> Option<Option<i32>> {
    match value {
        UpdateField::Unchanged => None,
        UpdateField::Clear => Some(None),
        UpdateField::Set(tokens) => Some(to_db(Some(tokens))),
    }
}

/// Read a column value; non-positive persisted values mean unset.
pub fn from_db(value: Option<i32>) -> Option<u32> {
    value
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_and_overflow() {
        assert!(validate(Some(0)).is_err());
        assert!(validate(Some(i32::MAX as u32 + 1)).is_err());
        assert!(validate(Some(1)).is_ok());
        assert!(validate(None).is_ok());
        assert!(validate_update(&UpdateField::Set(0)).is_err());
        assert!(validate_update(&UpdateField::Clear).is_ok());
    }

    #[test]
    fn round_trips_and_ignores_invalid_rows() {
        assert_eq!(from_db(to_db(Some(4096))), Some(4096));
        assert_eq!(from_db(Some(0)), None);
        assert_eq!(from_db(Some(-5)), None);
        assert_eq!(update_to_db(UpdateField::Unchanged), None);
        assert_eq!(update_to_db(UpdateField::Clear), Some(None));
        assert_eq!(update_to_db(UpdateField::Set(512)), Some(Some(512)));
    }
}
