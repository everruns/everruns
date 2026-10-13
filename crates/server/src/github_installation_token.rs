//! Installation-token metadata from the authenticated GitHub mint response.

use chrono::{DateTime, Utc};
use everruns_contracts::runtime::GitHubInstallationScope;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Metadata comes from the authenticated installation mint response, never
/// from the requested scope or a token prefix.
#[derive(Deserialize)]
pub(crate) struct InstallationToken {
    pub token: String,
    expires_at: Option<DateTime<Utc>>,
    permissions: Option<BTreeMap<String, String>>,
    repositories: Option<Vec<Repository>>,
    repository_selection: Option<String>,
}

#[derive(Deserialize)]
struct Repository {
    full_name: String,
}

impl InstallationToken {
    pub(crate) fn into_credential(self) -> (String, Option<GitHubInstallationScope>) {
        let scope = match (self.expires_at, self.permissions, self.repositories) {
            (Some(expires_at), Some(permissions), Some(repositories))
                if self.repository_selection.as_deref() == Some("selected") =>
            {
                Some(GitHubInstallationScope {
                    repositories: repositories.into_iter().map(|r| r.full_name).collect(),
                    permissions,
                    expires_at,
                })
            }
            _ => None,
        };
        (self.token, scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_complete_selected_installation_response_attests_scope() {
        let value = json!({
            "token": "dummy-installation-token",
            "expires_at": "2026-10-13T02:00:00Z",
            "repository_selection": "selected",
            "repositories": [{"full_name": "acme/app"}],
            "permissions": {"contents": "read", "pull_requests": "read"}
        });
        let (_, scope) = serde_json::from_value::<InstallationToken>(value.clone())
            .unwrap()
            .into_credential();
        assert_eq!(scope.unwrap().repositories, vec!["acme/app"]);
        for field in [
            "expires_at",
            "permissions",
            "repositories",
            "repository_selection",
        ] {
            let mut incomplete = value.clone();
            incomplete.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<InstallationToken>(incomplete)
                    .unwrap()
                    .into_credential()
                    .1
                    .is_none()
            );
        }
        let mut all = value;
        all["repository_selection"] = json!("all");
        assert!(
            serde_json::from_value::<InstallationToken>(all)
                .unwrap()
                .into_credential()
                .1
                .is_none()
        );
    }
}
