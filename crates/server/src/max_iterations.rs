//! max_iterations conversion helpers.
//!
//! Security: database storage uses `INTEGER` (`i32`), while API/domain uses
//! `usize`. Never use lossy `as` casts for this field.

use anyhow::Context;

/// Convert user/API value to DB value with checked bounds.
pub fn to_db(value: Option<usize>) -> anyhow::Result<Option<i32>> {
    value
        .map(|v| i32::try_from(v).context("max_iterations exceeds supported limit (2_147_483_647)"))
        .transpose()
}

/// Convert DB value to runtime/API value safely.
///
/// Negative/zero values are treated as invalid persisted state and ignored.
pub fn from_db(value: Option<i32>) -> Option<usize> {
    value.and_then(|v| usize::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_db_rejects_overflow() {
        let too_large = (i32::MAX as usize) + 1;
        assert!(to_db(Some(too_large)).is_err());
    }

    #[test]
    fn from_db_rejects_negative() {
        assert_eq!(from_db(Some(-1)), None);
    }
}
