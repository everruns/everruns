//! GitHub tokens for git inside a Daytona sandbox.

use everruns_contracts::runtime::mcp_server::McpServerActsAs;
use everruns_contracts::runtime::{GitHubAccess, ToolContext};
use tracing::debug;

/// What git in a sandbox asks GitHub for: contents read and write (clone,
/// pull, push), for one repository when it is known, acting as the person
/// when they connected GitHub and the agent otherwise.
fn sandbox_git_access(repository: Option<&str>) -> GitHubAccess {
    let access = GitHubAccess::new(McpServerActsAs::UserOrService, &[("contents", "write")]);
    match repository {
        Some(repository) => access.for_repository(repository),
        None => access,
    }
}

/// Resolve GitHub token lazily from connections, with session secret fallback.
///
/// THREAT[TM-GITHUB-006]: the token is cut down to contents access, and to
/// `repository` when the caller knows it, so a token left on sandbox disk
/// cannot open issues, approve pull requests or reach other repositories.
pub(super) async fn get_github_token(
    context: &ToolContext,
    repository: Option<&str>,
) -> Option<String> {
    // Try lazy resolution from connections (preferred: always fresh)
    if let Some(ref resolver) = context.connection_resolver {
        match resolver
            .get_github_token(context.session_id, &sandbox_git_access(repository))
            .await
        {
            Ok(Some(credential)) if !credential.token.is_empty() => return Some(credential.token),
            Ok(_) => {}
            Err(e) => debug!("Connection resolver failed: {e}"),
        }
    }

    // Fallback: session secret (for backward compat with pre-injected tokens)
    if let Some(ref storage) = context.storage_store {
        match storage.get_secret(context.session_id, "GITHUB_TOKEN").await {
            Ok(Some(token)) if !token.is_empty() => return Some(token),
            Ok(_) => {}
            Err(e) => debug!("No GITHUB_TOKEN session secret: {e}"),
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THREAT[TM-GITHUB-006]: sandbox git gets contents access only, and only
    /// for the cloned repository when it is known.
    #[test]
    fn sandbox_git_access_is_contents_only_and_narrowed_to_the_clone() {
        let access = sandbox_git_access(Some("acme/app"));
        assert_eq!(access.acts_as, McpServerActsAs::UserOrService);
        assert_eq!(access.repositories, vec!["acme/app".to_string()]);
        assert_eq!(
            access.permissions.into_iter().collect::<Vec<_>>(),
            vec![("contents".to_string(), "write".to_string())]
        );
        assert!(sandbox_git_access(None).repositories.is_empty());
    }
}
