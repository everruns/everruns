// CLI credential management
//
// Decision: Credentials stored via dirs::config_dir()/everruns/credentials.json
// (Linux: ~/.config/everruns/, macOS: ~/Library/Application Support/everruns/)
// Decision: Multi-profile support from day one
// Decision: EVERRUNS_API_KEY env var always takes precedence
// Decision: File permissions 0600 on Unix

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Credential store (multi-profile)
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CredentialStore {
    /// Named profiles
    pub profiles: HashMap<String, Profile>,
    /// Active profile name
    pub current_profile: String,
}

/// A single profile (one server + credentials)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub api_url: String,
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
}

/// Resolved credentials (from env var or credential file)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedCredentials {
    pub api_key: String,
    pub api_url: String,
    pub org_id: Option<String>,
}

impl CredentialStore {
    /// Load from the default credentials file
    pub fn load() -> Result<Self> {
        let path = credentials_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let store: Self = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(store)
    }

    /// Save to the default credentials file
    pub fn save(&self) -> Result<()> {
        let path = credentials_path()?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }

        let content = serde_json::to_string_pretty(self)?;
        fs::write(&path, &content)
            .with_context(|| format!("Failed to write {}", path.display()))?;

        // Set file permissions to 0600 on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    /// Get the current profile
    pub fn current_profile(&self) -> Option<&Profile> {
        let name = if self.current_profile.is_empty() {
            "default"
        } else {
            &self.current_profile
        };
        self.profiles.get(name)
    }

    /// Set a profile
    pub fn set_profile(&mut self, name: &str, profile: Profile) {
        self.profiles.insert(name.to_string(), profile);
        if self.current_profile.is_empty() {
            self.current_profile = name.to_string();
        }
    }

    /// Remove a profile
    pub fn remove_profile(&mut self, name: &str) -> bool {
        self.profiles.remove(name).is_some()
    }

    /// Update org_id in the current profile
    pub fn set_org(&mut self, org_id: &str) -> Result<()> {
        let name = if self.current_profile.is_empty() {
            "default".to_string()
        } else {
            self.current_profile.clone()
        };
        let profile = self
            .profiles
            .get_mut(&name)
            .ok_or_else(|| anyhow!("No profile '{}' found. Run `everruns login` first.", name))?;
        profile.org_id = Some(org_id.to_string());
        Ok(())
    }
}

/// Resolve credentials with priority: CLI flag > env var > credential file
pub fn resolve_credentials(
    cli_api_key: Option<&str>,
    cli_api_url: Option<&str>,
    profile_name: Option<&str>,
) -> Result<ResolvedCredentials> {
    let default_url = "https://app.everruns.com/api";

    let env_key = cli_api_key
        .map(|s| s.to_string())
        .or_else(|| std::env::var("EVERRUNS_API_KEY").ok());
    let env_url = cli_api_url
        .map(|s| s.to_string())
        .or_else(|| std::env::var("EVERRUNS_API_URL").ok());

    if let Some(api_key) = env_key {
        return Ok(ResolvedCredentials {
            api_key,
            api_url: env_url.unwrap_or_else(|| default_url.to_string()),
            org_id: None,
        });
    }

    // 2. Credential file
    let store = CredentialStore::load().unwrap_or_default();
    let profile_key = profile_name.map(|s| s.to_string()).unwrap_or_else(|| {
        if store.current_profile.is_empty() {
            "default".to_string()
        } else {
            store.current_profile.clone()
        }
    });

    if let Some(profile) = store.profiles.get(&profile_key) {
        return Ok(ResolvedCredentials {
            api_key: profile.api_key.clone(),
            api_url: env_url.unwrap_or_else(|| profile.api_url.clone()),
            org_id: profile.org_id.clone(),
        });
    }

    Err(anyhow!(
        "Not logged in. Run `everruns login` or set EVERRUNS_API_KEY environment variable."
    ))
}

/// Path to the credentials file
pub fn credentials_path() -> Result<PathBuf> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| anyhow!("Could not determine config directory"))?;
    Ok(config_dir.join("everruns").join("credentials.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_store_roundtrip() {
        let mut store = CredentialStore::default();
        store.set_profile(
            "default",
            Profile {
                api_url: "http://localhost:9300/api".to_string(),
                api_key: "evr_test123".to_string(),
                org_id: Some("org_abc".to_string()),
                user_email: Some("test@example.com".to_string()),
                user_name: Some("Test User".to_string()),
            },
        );
        assert_eq!(store.current_profile, "default");
        assert!(store.current_profile().is_some());

        let json = serde_json::to_string(&store).unwrap();
        let parsed: CredentialStore = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.current_profile, "default");
        let profile = parsed.profiles.get("default").unwrap();
        assert_eq!(profile.api_key, "evr_test123");
        assert_eq!(profile.org_id, Some("org_abc".to_string()));
    }

    #[test]
    fn test_credential_store_remove_profile() {
        let mut store = CredentialStore::default();
        store.set_profile(
            "test",
            Profile {
                api_url: "http://localhost/api".to_string(),
                api_key: "evr_key".to_string(),
                org_id: None,
                user_email: None,
                user_name: None,
            },
        );
        assert!(store.remove_profile("test"));
        assert!(!store.remove_profile("test"));
    }

    #[test]
    fn test_credential_store_set_org() {
        let mut store = CredentialStore::default();
        store.set_profile(
            "default",
            Profile {
                api_url: "http://localhost/api".to_string(),
                api_key: "evr_key".to_string(),
                org_id: None,
                user_email: None,
                user_name: None,
            },
        );
        store.set_org("org_new").unwrap();
        assert_eq!(
            store.profiles.get("default").unwrap().org_id,
            Some("org_new".to_string())
        );
    }

    #[test]
    fn test_resolve_credentials_env_var() {
        // This test relies on env vars not being set in CI
        // Just test the fallback path
        let result = resolve_credentials(Some("evr_explicit"), None, None);
        assert!(result.is_ok());
        let creds = result.unwrap();
        assert_eq!(creds.api_key, "evr_explicit");
    }

    #[test]
    fn test_credentials_path() {
        let path = credentials_path().unwrap();
        assert!(path.to_str().unwrap().contains("everruns"));
        assert!(path.to_str().unwrap().ends_with("credentials.json"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_credentials_path_macos_uses_application_support() {
        let path = credentials_path().unwrap();
        let expected_prefix =
            dirs::config_dir().expect("dirs::config_dir() should succeed on macOS");
        assert!(
            path.starts_with(&expected_prefix),
            "macOS credentials path should start with {}, got: {}",
            expected_prefix.display(),
            path.display()
        );
        assert!(
            path.to_string_lossy()
                .contains("Library/Application Support"),
            "macOS credentials path should use ~/Library/Application Support, got: {}",
            path.display()
        );
    }
}
