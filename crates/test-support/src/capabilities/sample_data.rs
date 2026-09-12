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

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, MountDirectoryBuilder, MountPoint,
};

pub const SAMPLE_DATA_CAPABILITY_ID: &str = "sample_data";

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
        SAMPLE_DATA_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Sample Data"
    }

    fn description(&self) -> &str {
        "Mounts sample data files in the session filesystem for demonstration and testing. Includes example JSON, YAML, and documentation files."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Зразкові дані",
            "Монтує файли зі зразковими даними у файлову систему сесії для демонстрації та тестування. Містить приклади JSON, YAML і файлів документації.",
        )]
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
            "Read-only sample files are mounted at `/samples` (`users.json`, `config.yaml`, `README.md`) for demos, tests, and templates.",
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
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capability_types::MountSource;

    #[test]
    fn mounted_sample_files_are_readonly_text_with_usable_user_records() {
        let mounts = SampleDataCapability.mounts();
        assert_eq!(mounts.len(), 1);
        let mount = &mounts[0];
        assert_eq!(mount.path, "/samples");
        assert!(mount.is_readonly());
        assert_eq!(mount.capability_id, "sample_data");
        let MountSource::InlineDirectory { entries } = &mount.source else {
            panic!("expected directory")
        };
        let mut names = entries.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, ["README.md", "config.yaml", "users.json"]);
        for entry in entries.values() {
            let MountSource::InlineFile { content, encoding } = &entry.source else {
                panic!("expected inline text")
            };
            assert_eq!(encoding, "text");
            assert!(!content.is_empty());
        }
        let MountSource::InlineFile { content, .. } = &entries["users.json"].source else {
            unreachable!()
        };
        let users: serde_json::Value = serde_json::from_str(content).unwrap();
        let users = users.as_array().unwrap();
        assert!(!users.is_empty());
        let mut ids = std::collections::HashSet::new();
        for user in users {
            assert!(ids.insert(user["id"].as_u64().unwrap()));
            assert!(!user["name"].as_str().unwrap().is_empty());
            assert!(user["email"].as_str().unwrap().contains('@'));
            assert!(matches!(user["role"].as_str(), Some("admin" | "user")));
            assert!(user["active"].is_boolean());
        }
    }
}
