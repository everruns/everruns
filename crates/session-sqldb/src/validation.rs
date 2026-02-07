// Database name validation

use crate::error::SqlDbError;

/// Validate a database name.
///
/// Rules:
/// - Must start with letter or underscore
/// - Only alphanumeric and underscore characters
/// - 1-64 characters long
pub fn validate_database_name(name: &str) -> Result<(), SqlDbError> {
    if name.is_empty() {
        return Err(SqlDbError::InvalidDatabaseName(
            "database name cannot be empty".to_string(),
        ));
    }

    if name.len() > 64 {
        return Err(SqlDbError::InvalidDatabaseName(
            "database name cannot exceed 64 characters".to_string(),
        ));
    }

    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(SqlDbError::InvalidDatabaseName(
            "database name must start with a letter or underscore".to_string(),
        ));
    }

    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(SqlDbError::InvalidDatabaseName(
            "database name can only contain letters, numbers, and underscores".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_names() {
        assert!(validate_database_name("main").is_ok());
        assert!(validate_database_name("analytics").is_ok());
        assert!(validate_database_name("user_data").is_ok());
        assert!(validate_database_name("_private").is_ok());
        assert!(validate_database_name("DB1").is_ok());
        assert!(validate_database_name("a").is_ok());
        assert!(validate_database_name("a_b_c_123").is_ok());
    }

    #[test]
    fn test_empty_name() {
        assert!(matches!(
            validate_database_name(""),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
    }

    #[test]
    fn test_too_long() {
        let long_name = "a".repeat(65);
        assert!(matches!(
            validate_database_name(&long_name),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
        // Exactly 64 is OK
        let max_name = "a".repeat(64);
        assert!(validate_database_name(&max_name).is_ok());
    }

    #[test]
    fn test_starts_with_number() {
        assert!(matches!(
            validate_database_name("1abc"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
    }

    #[test]
    fn test_special_characters() {
        assert!(matches!(
            validate_database_name("my-db"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
        assert!(matches!(
            validate_database_name("my.db"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
        assert!(matches!(
            validate_database_name("my db"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
        assert!(matches!(
            validate_database_name("my/db"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
    }

    #[test]
    fn test_sql_injection_in_name() {
        assert!(matches!(
            validate_database_name("'; DROP TABLE --"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
        assert!(matches!(
            validate_database_name("db; SELECT *"),
            Err(SqlDbError::InvalidDatabaseName(_))
        ));
    }
}
