//! Sample Data Capability
//!
//! A demonstration capability that shows how capabilities can mount files and
//! directories in the session filesystem. This capability mounts sample data
//! files that agents can read during execution.
//!
//! Mount points:
//! - `/samples/users.json` - Sample user data
//! - `/samples/config.yaml` - Sample configuration
//! - `/samples/README.md` - Documentation about the sample files

use super::{Capability, CapabilityId, CapabilityStatus, MountDirectoryBuilder, MountPoint};

/// Sample Data capability - demonstrates capability mounting
pub struct SampleDataCapability;

impl SampleDataCapability {
    /// Sample users JSON data
    const USERS_JSON: &'static str = r#"[
  {
    "id": 1,
    "name": "Alice Johnson",
    "email": "alice@example.com",
    "role": "admin",
    "active": true
  },
  {
    "id": 2,
    "name": "Bob Smith",
    "email": "bob@example.com",
    "role": "user",
    "active": true
  },
  {
    "id": 3,
    "name": "Carol Davis",
    "email": "carol@example.com",
    "role": "user",
    "active": false
  }
]"#;

    /// Sample configuration YAML
    const CONFIG_YAML: &'static str = r#"# Sample Configuration
application:
  name: "Sample App"
  version: "1.0.0"
  debug: false

database:
  host: localhost
  port: 5432
  name: sample_db
  pool_size: 10

features:
  enable_notifications: true
  enable_analytics: false
  max_upload_size_mb: 50

logging:
  level: info
  format: json
"#;

    /// README content
    const README: &'static str = r#"# Sample Data Files

This directory contains sample data files mounted by the Sample Data capability.

## Available Files

- `users.json` - A JSON array of sample user records with fields:
  - `id`: Unique user identifier
  - `name`: User's full name
  - `email`: User's email address
  - `role`: User role (admin/user)
  - `active`: Whether the account is active

- `config.yaml` - A sample YAML configuration file demonstrating:
  - Application settings
  - Database configuration
  - Feature flags
  - Logging settings

## Usage

These files are read-only and provided for demonstration purposes.
You can read them using the file system tools to see examples of
data formats commonly used in applications.

## Notes

- Files in this directory cannot be modified (read-only mount)
- Use `read_file` tool to read the contents
- Use `list_directory` to see available files
"#;
}

impl Capability for SampleDataCapability {
    fn id(&self) -> &str {
        CapabilityId::SAMPLE_DATA
    }

    fn name(&self) -> &str {
        "Sample Data"
    }

    fn description(&self) -> &str {
        "Mounts sample data files in the session filesystem for demonstration and testing. Includes example JSON, YAML, and documentation files."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("database")
    }

    fn category(&self) -> Option<&str> {
        Some("Data")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"Sample data files are available in the /samples directory. These include:
- /samples/users.json - Example user data in JSON format
- /samples/config.yaml - Example configuration in YAML format
- /samples/README.md - Documentation about the sample files

These files are read-only and can be used for learning, testing, or as templates."#,
        )
    }

    fn mounts(&self) -> Vec<MountPoint> {
        let samples_dir = MountDirectoryBuilder::new()
            .file("users.json", Self::USERS_JSON)
            .file("config.yaml", Self::CONFIG_YAML)
            .file("README.md", Self::README)
            .build();

        vec![MountPoint::readonly("/samples", samples_dir, self.id())]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        // Sample Data depends on Session File System for file operations
        vec![CapabilityId::FILE_SYSTEM]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_types::MountSource;

    #[test]
    fn test_capability_metadata() {
        let cap = SampleDataCapability;
        assert_eq!(cap.id(), CapabilityId::SAMPLE_DATA);
        assert_eq!(cap.name(), "Sample Data");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("database"));
        assert_eq!(cap.category(), Some("Data"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SampleDataCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("/samples"));
        assert!(prompt.contains("users.json"));
        assert!(prompt.contains("config.yaml"));
    }

    #[test]
    fn test_capability_has_no_tools() {
        let cap = SampleDataCapability;
        assert!(cap.tools().is_empty());
    }

    #[test]
    fn test_capability_has_mounts() {
        let cap = SampleDataCapability;
        let mounts = cap.mounts();

        assert_eq!(mounts.len(), 1);

        let mount = &mounts[0];
        assert_eq!(mount.path, "/samples");
        assert!(mount.is_readonly());
        assert_eq!(mount.capability_id, "sample_data");

        // Check directory structure
        match &mount.source {
            MountSource::InlineDirectory { entries } => {
                assert_eq!(entries.len(), 3);
                assert!(entries.contains_key("users.json"));
                assert!(entries.contains_key("config.yaml"));
                assert!(entries.contains_key("README.md"));
            }
            _ => panic!("Expected InlineDirectory"),
        }
    }

    #[test]
    fn test_sample_users_json_is_valid() {
        let json: serde_json::Value =
            serde_json::from_str(SampleDataCapability::USERS_JSON).unwrap();
        assert!(json.is_array());
        let users = json.as_array().unwrap();
        assert_eq!(users.len(), 3);
    }

    #[test]
    fn test_sample_config_yaml_is_valid() {
        // Just check it's not empty and looks like YAML
        assert!(SampleDataCapability::CONFIG_YAML.contains("application:"));
        assert!(SampleDataCapability::CONFIG_YAML.contains("database:"));
    }

    #[test]
    fn test_capability_depends_on_file_system() {
        let cap = SampleDataCapability;
        let deps = cap.dependencies();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], CapabilityId::FILE_SYSTEM);
    }
}
