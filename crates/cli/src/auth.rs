// CLI credential management
//
// Decision: Credentials stored via crate::user_dirs::config_dir()/everruns/credentials.json
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
    let config_dir = crate::user_dirs::config_dir()
        .ok_or_else(|| anyhow!("Could not determine config directory"))?;
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
        assert_eq!(store.current_profile().unwrap().api_key, "evr_test123");

        let json = serde_json::to_string(&store).unwrap();
        let parsed: CredentialStore = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.current_profile, "default");
        let profile = parsed.profiles.get("default").unwrap();
        assert_eq!(profile.api_url, "http://localhost:9300/api");
        assert_eq!(profile.api_key, "evr_test123");
        assert_eq!(profile.org_id, Some("org_abc".to_string()));
        assert_eq!(profile.user_email.as_deref(), Some("test@example.com"));
        assert_eq!(profile.user_name.as_deref(), Some("Test User"));
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
        assert!(store.current_profile().is_none());
        assert!(!store.remove_profile("test"));
    }

    #[test]
    fn test_credential_store_set_org() {
        let mut store = CredentialStore::default();
        assert!(store.set_org("org_new").is_err());
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
        store.set_profile("work", store.profiles["default"].clone());
        store.current_profile = "work".into();
        store.set_org("org_new").unwrap();
        assert_eq!(
            store.profiles.get("work").unwrap().org_id,
            Some("org_new".to_string())
        );
        assert!(store.profiles["default"].org_id.is_none());
        store.current_profile.clear();
        store.set_org("org_fallback").unwrap();
        assert_eq!(
            store.profiles["default"].org_id.as_deref(),
            Some("org_fallback")
        );
        assert_eq!(store.profiles["work"].org_id.as_deref(), Some("org_new"));
    }

    #[test]
    fn test_resolve_credentials_explicit_flags() {
        let creds = resolve_credentials(
            Some("evr_explicit"),
            Some("https://explicit.example/api"),
            None,
        )
        .unwrap();
        assert_eq!(creds.api_key, "evr_explicit");
        assert_eq!(creds.api_url, "https://explicit.example/api");
    }

    #[test]
    fn test_credentials_path() {
        match crate::user_dirs::config_dir() {
            Some(base) => assert_eq!(
                credentials_path().unwrap(),
                base.join("everruns").join("credentials.json")
            ),
            None => assert!(credentials_path().is_err()),
        }
    }
}
