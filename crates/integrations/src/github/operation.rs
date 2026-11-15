//! One vendor operation with host-owned credential acquisition.

use async_trait::async_trait;
use everruns_contracts::runtime::{
    GitHubAccess, tool_context::ToolContext, tools::ToolExecutionResult,
};

use super::{GitHubActsAs, client::GitHubClient, tools};

#[async_trait]
pub(super) trait GitHubApi: Send + Sync {
    async fn client(
        &self,
        repository: &str,
        permissions: &[(&str, &str)],
    ) -> Result<GitHubClient, ToolExecutionResult>;
}

pub(super) struct HostedGitHubApi<'a> {
    pub context: &'a ToolContext,
    pub acts_as: GitHubActsAs,
}

#[async_trait]
impl GitHubApi for HostedGitHubApi<'_> {
    async fn client(
        &self,
        repository: &str,
        permissions: &[(&str, &str)],
    ) -> Result<GitHubClient, ToolExecutionResult> {
        tools::enforce_github_network_access(self.context)?;
        let mut access = GitHubAccess::new(self.acts_as.into(), permissions);
        if !repository.is_empty() {
            access = access.for_repository(repository);
        }
        tools::get_github_token(self.context, &access)
            .await
            .map(tools::github_client)
    }
}
