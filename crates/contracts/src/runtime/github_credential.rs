//! Scope attested by the trusted GitHub installation-token minting boundary.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Only installation minting may supply this evidence. Personal credentials
/// and session secrets have no installation scope, irrespective of token text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubInstallationScope {
    pub repositories: Vec<String>,
    pub permissions: BTreeMap<String, String>,
    pub expires_at: DateTime<Utc>,
}

impl GitHubInstallationScope {
    /// THREAT[TM-GITHUB-007]: enforce the accepted sandbox exposure contract.
    pub fn permits_checkout(&self, repository: &str, now: DateTime<Utc>) -> bool {
        self.repositories.len() == 1
            && self.repositories[0].eq_ignore_ascii_case(repository)
            && self.permissions.get("contents").map(String::as_str) == Some("read")
            && self.permissions.get("pull_requests").map(String::as_str) == Some("read")
            && self.permissions.iter().all(|(name, level)| {
                matches!(name.as_str(), "contents" | "pull_requests" | "metadata")
                    && level == "read"
            })
            && self.expires_at > now
            && self.expires_at <= now + Duration::hours(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_admission_checks_repository_permissions_and_lifetime() {
        let now = DateTime::from_timestamp(1_000, 0).unwrap();
        let scope = GitHubInstallationScope {
            repositories: vec!["acme/app".into()],
            permissions: BTreeMap::from([
                ("contents".into(), "read".into()),
                ("pull_requests".into(), "read".into()),
                ("metadata".into(), "read".into()),
            ]),
            expires_at: now + Duration::minutes(30),
        };
        assert!(scope.permits_checkout("Acme/App", now));
        assert!(!scope.permits_checkout("other/app", now));
        let mut changed = scope.clone();
        changed.repositories.push("acme/other".into());
        assert!(!changed.permits_checkout("acme/app", now));
        changed = scope.clone();
        changed
            .permissions
            .insert("contents".into(), "write".into());
        assert!(!changed.permits_checkout("acme/app", now));
        changed = scope.clone();
        changed.permissions.insert("issues".into(), "read".into());
        assert!(!changed.permits_checkout("acme/app", now));
        changed = scope.clone();
        changed.permissions.remove("pull_requests");
        assert!(!changed.permits_checkout("acme/app", now));
        changed = scope.clone();
        changed.expires_at = now;
        assert!(!changed.permits_checkout("acme/app", now));
        changed.expires_at = now + Duration::hours(2);
        assert!(!changed.permits_checkout("acme/app", now));
        let wire = serde_json::to_value(&scope).unwrap();
        assert_eq!(
            serde_json::from_value::<GitHubInstallationScope>(wire).unwrap(),
            scope
        );
    }
}
