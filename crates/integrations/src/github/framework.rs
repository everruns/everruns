//! Application-owned GitHub credentials; protocol and vendor operations are shared with Server.

use std::{collections::BTreeMap, fmt, marker::PhantomData, sync::Arc};

use async_trait::async_trait;
use everruns_contracts::{
    capability::{
        CapabilitySpec, IntoCapability,
        definition::{self, Handler, JsonSchema},
    },
    runtime::tools::{Tool, ToolExecutionResult},
};
use serde_json::Value;

use super::{client::GitHubClient, operation::GitHubApi};

/// The minimum GitHub access one operation requests.
///
/// Application providers may mint repository-scoped installation tokens. A static
/// personal token keeps its original permissions; this request cannot narrow it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GitHubTokenRequest {
    /// One `owner/repo`, or `None` for repository discovery/search.
    pub repository: Option<String>,
    /// GitHub App permission names and their requested `read` or `write` level.
    pub permissions: BTreeMap<String, String>,
}

/// Application-owned token acquisition, called on every GitHub tool invocation.
///
/// Implement refresh or installation-token minting here. Provider errors are
/// deliberately replaced with a generic error so secrets cannot reach a model.
#[async_trait]
pub trait GitHubTokenProvider: Send + Sync {
    /// Return the token to use for this operation, or an error.
    async fn token(&self, request: &GitHubTokenRequest) -> Result<String, String>;
}

struct StaticToken(String);

#[async_trait]
impl GitHubTokenProvider for StaticToken {
    async fn token(&self, _request: &GitHubTokenRequest) -> Result<String, String> {
        Ok(self.0.clone())
    }
}

/// GitHub tools for a Framework agent, read-only by default.
///
/// Enable the Framework's `github` Cargo feature. Credentials stay private to
/// this value and never enter serialized agent configuration or debug output.
#[derive(Clone)]
pub struct GitHub {
    api: Arc<ApplicationGitHubApi>,
    writes: bool,
    pull_requests: bool,
    merge: bool,
    private_issues_only: bool,
}

impl GitHub {
    /// Use an application-owned token. No environment variables are read.
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_token_provider(StaticToken(token.into()))
    }

    /// Read `GITHUB_TOKEN` once at startup. Empty credentials are rejected.
    pub fn from_env() -> Result<Self, std::env::VarError> {
        let token = std::env::var(super::GITHUB_TOKEN_SECRET)?;
        if token.trim().is_empty() {
            return Err(std::env::VarError::NotPresent);
        }
        Ok(Self::new(token))
    }

    /// Resolve credentials per operation, for rotating or short-lived tokens.
    pub fn with_token_provider(provider: impl GitHubTokenProvider + 'static) -> Self {
        Self {
            api: Arc::new(ApplicationGitHubApi {
                provider: Arc::new(provider),
                api_base: super::GITHUB_API_BASE.into(),
            }),
            writes: false,
            pull_requests: false,
            merge: false,
            private_issues_only: false,
        }
    }

    /// Add comment, review, and issue tools. Off by default.
    ///
    /// Reviews can comment or request changes, never approve a pull request.
    pub fn allow_writes(mut self, allow: bool) -> Self {
        self.writes = allow;
        self
    }

    /// Offer PR creation as well. Requires `allow_writes(true)`; drafts are the default.
    pub fn allow_pull_requests(mut self, allow: bool) -> Self {
        self.pull_requests = allow;
        self
    }

    /// Offer marking a pull request ready and merging it. Requires
    /// `allow_writes(true)`. GitHub must report the pull request ready to
    /// merge, and only the commit the agent names is merged.
    pub fn allow_merge(mut self, allow: bool) -> Self {
        self.merge = allow;
        self
    }

    /// Refuse to file issues in public repositories, enforced by the tool.
    pub fn private_issues_only(mut self, private_only: bool) -> Self {
        self.private_issues_only = private_only;
        self
    }

    /// Use a trusted API root, such as a GitHub Enterprise API or a test server.
    ///
    /// This endpoint receives credentials; never take it from model output.
    pub fn api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api = Arc::new(ApplicationGitHubApi {
            provider: self.api.provider.clone(),
            api_base: api_base.into().trim_end_matches('/').into(),
        });
        self
    }
}

impl fmt::Debug for GitHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHub")
            .field("allow_writes", &self.writes)
            .field("allow_pull_requests", &self.pull_requests)
            .field("allow_merge", &self.merge)
            .field("private_issues_only", &self.private_issues_only)
            .finish_non_exhaustive()
    }
}

struct ApplicationGitHubApi {
    provider: Arc<dyn GitHubTokenProvider>,
    api_base: String,
}

#[async_trait]
impl GitHubApi for ApplicationGitHubApi {
    async fn client(
        &self,
        repository: &str,
        permissions: &[(&str, &str)],
    ) -> Result<GitHubClient, ToolExecutionResult> {
        let request = GitHubTokenRequest {
            repository: (!repository.is_empty()).then(|| repository.to_string()),
            permissions: permissions
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        };
        let token =
            self.provider.token(&request).await.map_err(|_| {
                ToolExecutionResult::tool_error("Could not acquire GitHub credentials")
            })?;
        if token.trim().is_empty() {
            return Err(ToolExecutionResult::tool_error(
                "GitHub credentials are empty",
            ));
        }
        Ok(GitHubClient::with_base_url(token, self.api_base.clone()))
    }
}

impl IntoCapability for GitHub {
    fn into_capability(self) -> CapabilitySpec {
        let mut definition = definition::Definition::new(
            "github",
            "GitHub",
            "Read and search GitHub repositories and pull requests, with explicitly enabled write tools.",
        );
        macro_rules! add {
            ($input:ty, $tool:expr) => {
                definition = definition.tool(Adapter::<$input> {
                    api: self.api.clone(),
                    tool: Box::new($tool),
                    private_issues_only: self.private_issues_only,
                    input: PhantomData,
                });
            };
        }
        add!(
            PullRequest,
            super::pull_requests::GetGitHubPullRequestTool::default()
        );
        add!(
            PullDiff,
            super::pull_requests::GetGitHubPullRequestDiffTool::default()
        );
        add!(SearchCode, super::tools::SearchGitHubCodeTool);
        add!(ReadFile, super::tools::ReadGitHubFileTool);
        add!(SearchIssues, super::tools::SearchGitHubIssuesTool);
        if self.writes {
            add!(
                Comment,
                super::pull_requests::UpsertGitHubCommentTool::default()
            );
            add!(Review, super::reviews::SubmitGitHubReviewTool::default());
            add!(
                Issue,
                super::issues::UpsertGitHubIssueTool::new(self.private_issues_only)
            );
            if self.pull_requests {
                add!(
                    CreatePull,
                    super::fix_pull_requests::CreateGitHubPullRequestTool::default()
                );
            }
            if self.merge {
                add!(
                    MarkReady,
                    super::merge_pull_requests::MarkGitHubPullRequestReadyTool::default()
                );
                add!(
                    MergePull,
                    super::merge_pull_requests::MergeGitHubPullRequestTool::default()
                );
            }
        }
        definition.into()
    }
}

#[async_trait]
trait Operation: serde::de::DeserializeOwned + JsonSchema + Send + Sync + 'static {
    async fn run(self, api: &dyn GitHubApi, private_only: bool) -> ToolExecutionResult;
}

// Reuse the hosted tool's exact schema and complete vendor operation. The
// private input wrapper avoids publishing a second set of GitHub wire types.
macro_rules! operation {
    ($input:ident, $tool:expr, $run:path $(, $private:ident)?) => {
        #[derive(definition::Deserialize)]
        #[serde(transparent)]
        struct $input(Value);
        impl JsonSchema for $input {
            fn schema_name() -> std::borrow::Cow<'static, str> { stringify!($input).into() }
            fn json_schema(_generator: &mut definition::schemars::SchemaGenerator) -> definition::schemars::Schema {
                $tool.parameters_schema().try_into().unwrap_or_else(|_| false.into())
            }
        }
        #[async_trait]
        impl Operation for $input {
            async fn run(self, api: &dyn GitHubApi, _private_only: bool) -> ToolExecutionResult {
                $(let $private = _private_only;)?
                $run(api, self.0 $(, $private)?).await
            }
        }
    };
}
operation!(
    PullRequest,
    super::pull_requests::GetGitHubPullRequestTool::default(),
    super::pull_requests::get_pull_request
);
operation!(
    PullDiff,
    super::pull_requests::GetGitHubPullRequestDiffTool::default(),
    super::pull_requests::get_pull_request_diff
);
operation!(
    SearchCode,
    super::tools::SearchGitHubCodeTool,
    super::tools::search_code
);
operation!(
    ReadFile,
    super::tools::ReadGitHubFileTool,
    super::tools::read_file
);
operation!(
    SearchIssues,
    super::tools::SearchGitHubIssuesTool,
    super::tools::search_issues
);
operation!(
    Comment,
    super::pull_requests::UpsertGitHubCommentTool::default(),
    super::pull_requests::upsert_comment
);
operation!(
    Review,
    super::reviews::SubmitGitHubReviewTool::default(),
    super::reviews::submit_review
);
operation!(
    Issue,
    super::issues::UpsertGitHubIssueTool::new(false),
    super::issues::upsert_issue,
    private_only
);
operation!(
    CreatePull,
    super::fix_pull_requests::CreateGitHubPullRequestTool::default(),
    super::fix_pull_requests::create_pull_request
);
operation!(
    MarkReady,
    super::merge_pull_requests::MarkGitHubPullRequestReadyTool::default(),
    super::merge_pull_requests::mark_ready
);
operation!(
    MergePull,
    super::merge_pull_requests::MergeGitHubPullRequestTool::default(),
    super::merge_pull_requests::merge_pull_request
);

struct Adapter<I> {
    api: Arc<ApplicationGitHubApi>,
    tool: Box<dyn Tool>,
    private_issues_only: bool,
    input: PhantomData<I>,
}

#[async_trait]
impl<I: Operation> Handler for Adapter<I> {
    type Input = I;
    type Output = Value;
    type Error = definition::Error;
    fn name(&self) -> &str {
        self.tool.name()
    }
    fn description(&self) -> &str {
        self.tool.description()
    }
    fn hints(&self) -> definition::Hints {
        let hints = self.tool.hints();
        definition::Hints {
            readonly: hints.readonly,
            destructive: hints.destructive,
            idempotent: hints.idempotent,
            open_world: hints.open_world,
            long_running: hints.long_running,
            concurrency_class: hints.concurrency_class,
            ..Default::default()
        }
    }
    async fn execute(
        &self,
        input: I,
        _context: definition::Context,
    ) -> Result<Value, definition::Error> {
        match input.run(self.api.as_ref(), self.private_issues_only).await {
            ToolExecutionResult::Success(value) => Ok(value),
            ToolExecutionResult::ToolError(message) => {
                Err(definition::Error::user("github_error", message))
            }
            _ => Err(definition::Error::internal(
                "github_internal",
                "GitHub operation failed",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::runtime::Capability;
    use serde_json::json;

    #[test]
    fn framework_and_hosted_tools_share_protocol_and_hints() {
        let definition = GitHub::new("secret")
            .allow_writes(true)
            .allow_pull_requests(true)
            .allow_merge(true)
            .into_capability()
            .into_parts()
            .definition
            .unwrap();
        let mut hosted = super::super::GitHubCapability
            .tools_with_config(&json!({"allow_pull_requests":true, "allow_merge":true}));
        hosted.retain(|tool| tool.name() != "github_checkout_pull_request");
        hosted.extend([
            Box::new(super::super::tools::SearchGitHubCodeTool) as Box<dyn Tool>,
            Box::new(super::super::tools::ReadGitHubFileTool),
            Box::new(super::super::tools::SearchGitHubIssuesTool),
        ]);
        assert_eq!(definition.tools().len(), hosted.len());
        for tool in definition.tools() {
            let spec = tool.spec();
            let original = hosted
                .iter()
                .find(|tool| tool.name() == spec.name())
                .unwrap();
            assert_eq!(spec.description(), original.description());
            let mut schema = spec.input_schema().clone();
            schema.as_object_mut().unwrap().remove("$schema");
            schema.as_object_mut().unwrap().remove("title");
            assert_eq!(schema, original.parameters_schema(), "{}", spec.name());
            assert_eq!(spec.hints().readonly, original.hints().readonly);
            assert_eq!(spec.hints().idempotent, original.hints().idempotent);
        }
    }
}
