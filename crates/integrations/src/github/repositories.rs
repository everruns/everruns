//! `repositories`: the set of GitHub repositories a session works on, checked
//! out side by side in the sandbox at `/workspace/<name>`.
//!
//! Decisions:
//! - One repository is a list with one entry. There is no single-repository
//!   layout, so a task that grows to a second repository changes nothing.
//! - The sandbox never keeps a GitHub credential. Every clone, fetch and push
//!   is a tool call that mints a token for that one operation, hands it to the
//!   sandbox as a short-lived file (`SandboxWorkspace::exec_with_secret_file`)
//!   and passes it to git through a one-shot credential helper. `git push`
//!   typed into the shell has nothing to authenticate with, which is the
//!   point: what leaves the sandbox goes through `repo_push`.
//! - Access comes from the capability config, never from the sandbox. A
//!   repository is writable when its config entry says so (or when the agent
//!   added it and the add policy is `write`). The checkout's own `origin` is
//!   agent-editable, so `repo_push` pushes to the URL built from the
//!   repository name it authorized and mints the token for that repository.
//! - Admission needs installation-mint evidence, as the pull request checkout
//!   does: a clone token reads only the repositories asked for, a push token
//!   writes contents of exactly one. Personal tokens and session secrets are
//!   never delivered to the sandbox. THREAT[TM-GITHUB-009].
//! - Clones are shallow, not blobless. A blobless clone fetches file contents
//!   later, on an ordinary `git diff` or `git checkout`, and those fetches
//!   would fail without a credential.
//! - One work branch per session, the same name in every repository the
//!   session changes, so a change across repositories is easy to follow.

use std::collections::BTreeMap;

use async_trait::async_trait;
use everruns_contracts::capability::json_schema_for;
use everruns_contracts::capability::schemars::JsonSchema;
use everruns_contracts::runtime::GitHubAccess;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext,
};
use everruns_contracts::runtime::sandbox_workspace::{SandboxWorkspace, SandboxWorkspaceExt};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::ToolHints;
use everruns_contracts::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::GitHubActsAs;
use super::fix_pull_requests::is_valid_branch;
use super::tools::{enforce_github_network_access, is_valid_owner_repo, required_str};

pub const REPOSITORIES_CAPABILITY_ID: &str = "repositories";

const DEFAULT_DEPTH: u64 = 200;
const MAX_DEPTH: u64 = 10_000;
const MAX_REPOSITORIES: usize = 20;
const MAX_ERROR_BYTES: usize = 1_500;
const MAX_INSTRUCTIONS_BYTES: usize = 16 * 1024;
/// Session feature this capability contributes.
pub const REPOSITORIES_FEATURE: &str = "repositories";
pub(super) const STATUS_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_BRANCH_PREFIX: &str = "everruns/";
const DEFAULT_COMMIT_NAME: &str = "Everruns Agent";
/// `.invalid` is reserved: the address names no mailbox and no GitHub account.
const DEFAULT_COMMIT_EMAIL: &str = "agent@everruns.invalid";

/// Answers only for `github.com`, so a checkout whose git config redirects the
/// URL elsewhere gets no token. Passed with `-c`, so it lives on one git
/// command and nowhere else.
const CREDENTIAL_HELPER: &str = r#"!f() { test "$1" = get || exit 0; ok=0; while IFS= read -r line; do test "$line" = host=github.com && ok=1; done; test "$ok" = 1 || exit 0; echo username=x-access-token; printf 'password=%s\n' "$(cat "$EVR_SECRET_FILE")"; }; f"#;

/// What the session may do with a repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(crate = "everruns_contracts::capability::schemars")]
pub enum RepositoryAccess {
    /// Clone and fetch.
    #[default]
    Read,
    /// Clone, fetch and push the session's work branch.
    Write,
}

impl RepositoryAccess {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// Whether the agent may bring in repositories the config does not list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(crate = "everruns_contracts::capability::schemars")]
pub enum AddRepositories {
    /// Only the listed repositories.
    Off,
    /// The agent may add repositories to read.
    #[default]
    Read,
    /// The agent may add repositories and push its work branch to them.
    Write,
}

/// One repository of the session's set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(crate = "everruns_contracts::capability::schemars")]
pub struct RepositoryEntry {
    /// Repository in owner/repo format.
    #[schemars(regex(pattern = super::REPO_PATTERN))]
    pub repo: String,
    /// Branch to start from. The repository's default branch when omitted.
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// `read` (default) or `write`. Write lets the agent push its work branch.
    #[serde(default)]
    pub access: RepositoryAccess,
}

/// Settings for the `repositories` capability. An agent sets its usual
/// repositories; a session started with its own `repositories` config works on
/// that set instead.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(crate = "everruns_contracts::capability::schemars")]
pub struct RepositoriesConfig {
    /// Whose GitHub identity clones and pushes. Only a GitHub App installation
    /// can be narrowed to the repositories below, so `agent` (default) is the
    /// setting that works; a person's own sign-in is never handed to a sandbox.
    #[schemars(title = "Act on GitHub as")]
    pub acts_as: GitHubActsAs,
    /// Repositories checked out when the session starts work.
    #[schemars(title = "Repositories")]
    pub repositories: Vec<RepositoryEntry>,
    /// Whether the agent may add other repositories during a session, and what
    /// it may do with them. The agent's GitHub identity still decides which
    /// repositories it can reach at all.
    #[schemars(title = "Repositories the agent adds")]
    pub add_repositories: AddRepositories,
    /// Start of the work branch name. The session's short id is appended, and
    /// the same branch name is used in every repository.
    #[schemars(title = "Work branch prefix")]
    pub branch_prefix: String,
    /// How much history to fetch, in commits.
    #[schemars(title = "History depth", range(min = 1, max = 10000))]
    pub depth: u64,
    /// Author name on the agent's commits, used when the sandbox has no git
    /// identity of its own.
    #[schemars(title = "Commit author name")]
    pub commit_name: String,
    /// Author email on the agent's commits. For commits that show the agent's
    /// GitHub App as author, use the App's
    /// `<id>+<slug>[bot]@users.noreply.github.com` address.
    #[schemars(title = "Commit author email")]
    pub commit_email: String,
}

impl Default for RepositoriesConfig {
    fn default() -> Self {
        Self {
            acts_as: GitHubActsAs::default(),
            repositories: Vec::new(),
            add_repositories: AddRepositories::default(),
            branch_prefix: DEFAULT_BRANCH_PREFIX.to_string(),
            depth: DEFAULT_DEPTH,
            commit_name: DEFAULT_COMMIT_NAME.to_string(),
            commit_email: DEFAULT_COMMIT_EMAIL.to_string(),
        }
    }
}

impl RepositoriesConfig {
    /// Whether this configuration lets the agent push anywhere: a writable
    /// repository, or leave to add one. An agent's GitHub App needs the
    /// Contents write permission exactly then.
    pub fn can_push(&self) -> bool {
        self.add_repositories == AddRepositories::Write
            || self
                .repositories
                .iter()
                .any(|entry| entry.access == RepositoryAccess::Write)
    }

    /// [`Self::can_push`] for a stored capability config. A config that does
    /// not parse grants nothing.
    pub fn config_can_push(config: &Value) -> bool {
        Self::parse(config).is_ok_and(|config| config.can_push())
    }

    pub(crate) fn parse(config: &Value) -> Result<Self, String> {
        let parsed: Self = if config.is_null() {
            Self::default()
        } else {
            serde_json::from_value(config.clone())
                .map_err(|e| format!("Invalid repositories config: {e}"))?
        };
        parsed.validate()?;
        Ok(parsed)
    }

    fn validate(&self) -> Result<(), String> {
        if self.repositories.len() > MAX_REPOSITORIES {
            return Err(format!(
                "repositories lists more than {MAX_REPOSITORIES} entries"
            ));
        }
        let mut seen = Vec::new();
        for entry in &self.repositories {
            if !is_valid_owner_repo(&entry.repo) {
                return Err(format!("`{}` is not in owner/repo format", entry.repo));
            }
            if let Some(git_ref) = &entry.git_ref
                && !is_valid_branch(git_ref)
            {
                return Err(format!(
                    "`{git_ref}` is not a valid branch for {}",
                    entry.repo
                ));
            }
            let key = entry.repo.to_ascii_lowercase();
            if seen.contains(&key) {
                return Err(format!("{} is listed twice", entry.repo));
            }
            seen.push(key);
        }
        if !is_valid_branch(&format!("{}00000000", self.branch_prefix)) {
            return Err("branch_prefix does not make a valid branch name".to_string());
        }
        if !(1..=MAX_DEPTH).contains(&self.depth) {
            return Err(format!("depth must be from 1 to {MAX_DEPTH}"));
        }
        for (name, value) in [
            ("commit_name", &self.commit_name),
            ("commit_email", &self.commit_email),
        ] {
            let plain = !value.trim().is_empty()
                && value.len() <= 200
                && !value
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '<' | '>'));
            if !plain {
                return Err(format!(
                    "{name} must be a short line without `<`, `>` or control characters"
                ));
            }
        }
        Ok(())
    }

    fn entry(&self, repo: &str) -> Option<&RepositoryEntry> {
        self.repositories
            .iter()
            .find(|entry| entry.repo.eq_ignore_ascii_case(repo))
    }

    /// What the session may do with `repo`: its listed access, else what the
    /// add policy gives a repository the agent brought in.
    fn access(&self, repo: &str) -> Option<RepositoryAccess> {
        match self.entry(repo) {
            Some(entry) => Some(entry.access),
            None => match self.add_repositories {
                AddRepositories::Off => None,
                AddRepositories::Read => Some(RepositoryAccess::Read),
                AddRepositories::Write => Some(RepositoryAccess::Write),
            },
        }
    }

    /// The session's work branch: the prefix plus the session's short id.
    pub(super) fn work_branch(&self, session: SessionId) -> String {
        let id = session.uuid().simple().to_string();
        let short = &id[id.len().saturating_sub(8)..];
        format!("{}{short}", self.branch_prefix)
    }
}

/// The session's repository set and the tools that move code in and out.
pub struct RepositoriesCapability;

#[async_trait]
impl Capability for RepositoriesCapability {
    fn id(&self) -> &str {
        REPOSITORIES_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Repositories"
    }

    fn description(&self) -> &str {
        "Work on several GitHub repositories in one sandbox: check them out side by side, see what changed in each, and push one work branch per session. The sandbox never keeps a GitHub credential."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("github")
    }

    fn category(&self) -> Option<&str> {
        Some("Integrations")
    }

    /// Lets the session page offer its per-repository Changes tab.
    fn features(&self) -> Vec<&'static str> {
        vec![REPOSITORIES_FEATURE]
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        // Invalid config is rejected on write; anything that still fails here
        // gets the defaults: no listed repositories, added ones read-only.
        let config = RepositoriesConfig::parse(config).unwrap_or_default();
        let mut tools: Vec<Box<dyn Tool>> = vec![
            Box::new(RepoTool::new(RepoAction::Status, &config)),
            Box::new(RepoTool::new(RepoAction::Sync, &config)),
        ];
        if config.add_repositories != AddRepositories::Off {
            tools.push(Box::new(RepoTool::new(RepoAction::Add, &config)));
        }
        if config.can_push() {
            tools.push(Box::new(RepoTool::new(RepoAction::Push, &config)));
        }
        tools
    }

    async fn system_prompt_contribution_with_config(
        &self,
        ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        let config = RepositoriesConfig::parse(config).unwrap_or_default();
        Some(environment_block(&config, ctx.session_id))
    }

    fn system_prompt_preview(&self) -> Option<String> {
        Some(environment_block(
            &RepositoriesConfig::default(),
            SessionId::from_seed(0),
        ))
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json_schema_for::<RepositoriesConfig>())
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        RepositoriesConfig::parse(config).map(|_| ())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Репозиторії",
            "Робота з кількома репозиторіями GitHub в одній пісочниці: вони лежать поруч, \
             видно зміни в кожному, і на сесію є одна робоча гілка. Пісочниця не зберігає \
             облікових даних GitHub.",
        )]
    }
}

/// What the model is told about its repositories. Derived from the config, so
/// it cannot drift from what the tools enforce.
fn environment_block(config: &RepositoriesConfig, session: SessionId) -> String {
    let mut block = String::from("## Repositories\n\n");
    if config.repositories.is_empty() {
        block.push_str("No repositories are set for this session yet.");
    } else {
        block.push_str(
            "This session works on these GitHub repositories, each checked out in its own \
             directory under `/workspace`:\n\n",
        );
        for entry in &config.repositories {
            let name = directory_names(&entry.repo).0;
            block.push_str(&format!(
                "- `{}` at `/workspace/{name}` ({}{})\n",
                entry.repo,
                entry.access.as_str(),
                entry
                    .git_ref
                    .as_deref()
                    .map(|git_ref| format!(", from `{git_ref}`"))
                    .unwrap_or_default(),
            ));
        }
    }
    block.push_str(&format!(
        "\n\nCheck the repositories out before anything else: asking for their status does it \
         and shows each one's branch and changes. `/workspace` itself is not a repository. \
         Read a repository's `AGENTS.md` before changing it.\n\n\
         Writable repositories start on the work branch `{}`. Commit there with git as usual. \
         The sandbox holds no GitHub credential, so `git push`, `git fetch` and `git clone` \
         from the shell cannot reach private repositories: bring code in and out with the \
         repository tools. A change that spans repositories is one pull request per \
         repository; link them to each other in their descriptions.\n\n\
         Code navigation is exact inside one repository. Across repositories, a symbol's users \
         are found by searching for its name.",
        config.work_branch(session)
    ));
    block
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn remote_url(repo: &str) -> String {
    format!("https://github.com/{repo}.git")
}

/// `owner/repo` from a GitHub remote URL, in any of the forms git accepts.
fn repo_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    is_valid_owner_repo(rest).then(|| rest.to_string())
}

/// The directory a repository is checked out in, and the one used when that
/// name is taken by a different repository.
fn directory_names(repo: &str) -> (String, String) {
    let (owner, name) = repo.split_once('/').unwrap_or(("", repo));
    (name.to_string(), format!("{owner}-{name}"))
}

fn tail(text: &str) -> &str {
    let text = text.trim();
    let mut start = text.len().saturating_sub(MAX_ERROR_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

pub(super) fn sandbox(
    context: &ToolContext,
) -> Result<std::sync::Arc<dyn SandboxWorkspace>, ToolExecutionResult> {
    context
        .extension::<SandboxWorkspaceExt>()
        .map(|ext| ext.0.clone())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "The repository tools need a sandbox. This session has none: use an agent \
                 whose sandbox policy selects a full sandbox.",
            )
        })
}

/// A checkout as the sandbox reports it. Everything here is the agent's to
/// change, so it informs and never authorizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Checkout {
    pub(super) directory: String,
    repo: Option<String>,
    branch: String,
    head: String,
    changed_files: u64,
    base: String,
    ahead_of_base: Option<u64>,
    unpushed: Option<u64>,
    has_instructions: bool,
}

/// Lists every git checkout directly under the workspace root. One line per
/// checkout, tab-separated; values are printed by git, not by the agent.
const STATUS_SCRIPT: &str = r#"for d in */; do
  d=${d%/}
  [ -d "$d/.git" ] || continue
  url=$(git -C "$d" config --get remote.origin.url 2>/dev/null || true)
  branch=$(git -C "$d" symbolic-ref -q --short HEAD 2>/dev/null || true)
  head=$(git -C "$d" rev-parse -q --verify HEAD 2>/dev/null || true)
  changed=$(git -C "$d" status --porcelain 2>/dev/null | wc -l | tr -d ' ')
  base=$(git -C "$d" config --get everruns.base 2>/dev/null || true)
  ahead=$(git -C "$d" rev-list --count "refs/remotes/origin/$base..HEAD" 2>/dev/null || true)
  unpushed=$(git -C "$d" rev-list --count "refs/remotes/origin/$branch..HEAD" 2>/dev/null || true)
  if [ -f "$d/AGENTS.md" ]; then notes=1; else notes=0; fi
  printf 'checkout\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$d" "$url" "$branch" "$head" "$changed" "$base" "$ahead" "$unpushed" "$notes"
done
# A loop that ends on `continue` (an empty workspace, or a last directory that
# is not a checkout) reports failure in some shells.
exit 0
"#;

fn parse_status(stdout: &str) -> Vec<Checkout> {
    stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [
                "checkout",
                directory,
                url,
                branch,
                head,
                changed,
                base,
                ahead,
                unpushed,
                notes,
            ] = fields[..]
            else {
                return None;
            };
            Some(Checkout {
                directory: directory.to_string(),
                repo: repo_from_url(url),
                branch: branch.to_string(),
                head: head.to_string(),
                changed_files: changed.parse().unwrap_or(0),
                base: base.to_string(),
                ahead_of_base: ahead.parse().ok(),
                unpushed: unpushed.parse().ok(),
                has_instructions: notes == "1",
            })
        })
        .collect()
}

pub(super) async fn checkouts(
    sandbox: &dyn SandboxWorkspace,
    context: &ToolContext,
) -> Result<(String, Vec<Checkout>), ToolExecutionResult> {
    let root = sandbox.provider_root(context).await?;
    let response = sandbox
        .exec_raw(context, STATUS_SCRIPT, Some(&root), Some(STATUS_TIMEOUT_MS))
        .await?;
    if response.exit_code != 0 {
        return Err(ToolExecutionResult::tool_error(format!(
            "Could not list repositories (exit {}): {}",
            response.exit_code,
            tail(&response.stderr)
        )));
    }
    Ok((root, parse_status(&response.stdout)))
}

/// A token for one operation, admitted only with installation-mint evidence
/// that `admit` accepts. THREAT[TM-GITHUB-009].
async fn sandbox_token(
    context: &ToolContext,
    access: &GitHubAccess,
    admit: impl Fn(&everruns_contracts::runtime::GitHubInstallationScope) -> bool,
) -> Result<String, ToolExecutionResult> {
    enforce_github_network_access(context)?;
    if let Some(resolver) = &context.connection_resolver {
        let credential = resolver
            .get_github_token(context.session_id, access)
            .await
            // The cause stays server-side. The usual one is an App created
            // before the agent could push: GitHub refuses to mint a token
            // with a permission the App does not hold, so say where to look.
            .map_err(|_| {
                ToolExecutionResult::tool_error(format!(
                    "Could not get a GitHub token with Contents {level} access to {repositories}. \
                     If the agent's GitHub App does not hold the Contents {level} permission, \
                     grant it in the App's settings and accept it on the installation, then \
                     try again.",
                    level = contents_level(access),
                    repositories = access.repositories.join(", ")
                ))
            })?;
        if let Some(credential) = credential
            && !credential.token.trim().is_empty()
            && access
                .acts_as
                .resolution_order()
                .contains(&credential.acted_as)
            && credential
                .github_installation_scope
                .as_ref()
                .is_some_and(admit)
        {
            return Ok(credential.token);
        }
    }
    Err(ToolExecutionResult::tool_error(format!(
        "No GitHub App installation gives this agent {} access to {}. Connect a GitHub App to \
         the agent and install it on the repository (pushing also needs the App's Contents \
         write permission). A person's own sign-in and GITHUB_TOKEN session secrets are never \
         handed to a sandbox.",
        contents_level(access),
        access.repositories.join(", ")
    )))
}

fn contents_level(access: &GitHubAccess) -> &str {
    access
        .permissions
        .get("contents")
        .map(String::as_str)
        .unwrap_or("read")
}

/// One repository to clone.
struct CloneTarget {
    repo: String,
    git_ref: Option<String>,
    /// Start the work branch after cloning.
    writable: bool,
}

/// Clones `targets` side by side, in parallel. Holds no secret. Each target
/// reports one `result` line: `ok <cloned|present> <directory>` or
/// `error <reason>`.
fn clone_script(targets: &[CloneTarget], work_branch: &str, config: &RepositoriesConfig) -> String {
    let depth = config.depth;
    let helper = quote(CREDENTIAL_HELPER);
    let mut script = format!(
        r#"set -u
export GIT_TERMINAL_PROMPT=0
place() {{
  for d in "$2" "$3"; do
    if [ -d "$d/.git" ]; then
      [ "$(git -C "$d" config --get remote.origin.url 2>/dev/null)" = "$1" ] && {{ echo "$d"; return 0; }}
    elif [ ! -e "$d" ] || [ -z "$(ls -A "$d" 2>/dev/null)" ]; then
      echo "$d"; return 0
    fi
  done
  return 1
}}
one() {{
  d=$(place "$2" "$3" "$4") || {{ printf 'result\t%s\terror\tboth %s and %s are taken by something else\n' "$1" "$3" "$4"; return 0; }}
  state=present
  if [ ! -d "$d/.git" ]; then
    log=$(mktemp)
    if [ -n "$5" ]; then
      git -c credential.helper= -c credential.helper={helper} -c core.hooksPath=/dev/null clone -q --no-tags --depth={depth} --branch "$5" -- "$2" "$d" 2>"$log"
    else
      git -c credential.helper= -c credential.helper={helper} -c core.hooksPath=/dev/null clone -q --no-tags --depth={depth} -- "$2" "$d" 2>"$log"
    fi
    code=$?
    if [ "$code" != 0 ]; then
      printf 'result\t%s\terror\t%s\n' "$1" "$(tail -c 600 "$log" | tr '\n\t' '  ')"
      rm -f "$log"; rm -rf -- "$d"
      return 0
    fi
    rm -f "$log"
    git -C "$d" config everruns.base "$(git -C "$d" symbolic-ref -q --short HEAD)"
    if [ -z "$(git -C "$d" config --get user.email 2>/dev/null)" ]; then
      git -C "$d" config user.name {name}
      git -C "$d" config user.email {email}
    fi
    if [ "$6" = 1 ]; then git -C "$d" checkout -q -b {work}; fi
    state=cloned
  fi
  printf 'result\t%s\tok\t%s\t%s\n' "$1" "$state" "$d"
}}
"#,
        work = quote(work_branch),
        name = quote(&config.commit_name),
        email = quote(&config.commit_email),
    );
    for target in targets {
        let (name, alternative) = directory_names(&target.repo);
        script.push_str(&format!(
            "one {} {} {} {} {} {} &\n",
            quote(&target.repo),
            quote(&remote_url(&target.repo)),
            quote(&name),
            quote(&alternative),
            quote(target.git_ref.as_deref().unwrap_or_default()),
            u8::from(target.writable),
        ));
    }
    script.push_str("wait\n");
    script
}

/// What one clone reported: the directory, or why it failed.
type CloneResults = BTreeMap<String, Result<(String, bool), String>>;

fn parse_clone_results(stdout: &str) -> CloneResults {
    stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields[..] {
                ["result", repo, "ok", state, directory] => Some((
                    repo.to_ascii_lowercase(),
                    Ok((directory.to_string(), state == "cloned")),
                )),
                ["result", repo, "error", reason] => {
                    Some((repo.to_ascii_lowercase(), Err(reason.trim().to_string())))
                }
                _ => None,
            }
        })
        .collect()
}

/// Clone `targets`, one sandbox command per repository owner (an installation
/// token covers one owner). Returns one outcome per repository, by lowercase
/// name.
async fn clone_repositories(
    sandbox: &dyn SandboxWorkspace,
    context: &ToolContext,
    config: &RepositoriesConfig,
    targets: Vec<CloneTarget>,
) -> CloneResults {
    let mut by_owner: BTreeMap<String, Vec<CloneTarget>> = BTreeMap::new();
    for target in targets {
        let owner = target
            .repo
            .split('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        by_owner.entry(owner).or_default().push(target);
    }
    let work_branch = config.work_branch(context.session_id);
    let mut results = CloneResults::new();
    for targets in by_owner.into_values() {
        let repositories: Vec<String> = targets.iter().map(|target| target.repo.clone()).collect();
        let mut access = GitHubAccess::new(config.acts_as.into(), &[("contents", "read")]);
        access.repositories = repositories.clone();
        let fail = |results: &mut CloneResults, reason: String| {
            for repo in &repositories {
                results.insert(repo.to_ascii_lowercase(), Err(reason.clone()));
            }
        };
        let token = match sandbox_token(context, &access, |scope| {
            scope.permits_clone(&repositories, chrono::Utc::now())
        })
        .await
        {
            Ok(token) => token,
            Err(error) => {
                fail(&mut results, error_text(&error));
                continue;
            }
        };
        let script = clone_script(&targets, &work_branch, config);
        match sandbox
            .exec_with_secret_file(context, &script, sandbox.workspace_root(), &token)
            .await
        {
            Ok(response) => {
                let mut parsed = parse_clone_results(&response.stdout);
                for repo in &repositories {
                    let key = repo.to_ascii_lowercase();
                    let outcome = parsed.remove(&key).unwrap_or_else(|| {
                        Err(format!(
                            "clone did not report a result (exit {}): {}",
                            response.exit_code,
                            tail(&response.stderr)
                        ))
                    });
                    results.insert(key, outcome);
                }
            }
            Err(error) => fail(&mut results, error_text(&error)),
        }
    }
    results
}

fn error_text(result: &ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::ToolError(message) => message.clone(),
        other => format!("{other:?}"),
    }
}

/// A repository's `AGENTS.md`, cut to a size that fits a tool result. It is
/// the repository author's text, so it is returned as data under its own key.
async fn instructions(
    sandbox: &dyn SandboxWorkspace,
    context: &ToolContext,
    root: &str,
    directory: &str,
) -> Option<String> {
    let path = format!("{}/{directory}/AGENTS.md", root.trim_end_matches('/'));
    let bytes = sandbox.read_file_raw(context, &path).await.ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut end = text.len().min(MAX_INSTRUCTIONS_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut text = text[..end].to_string();
    if end < bytes.len() {
        text.push_str("\n[cut: read the file for the rest]");
    }
    Some(text)
}

pub(super) fn checkout_json(
    checkout: &Checkout,
    config: &RepositoriesConfig,
    work_branch: &str,
) -> Value {
    let access = checkout
        .repo
        .as_deref()
        .and_then(|repo| config.access(repo));
    json!({
        "repo": checkout.repo,
        "path": format!("/workspace/{}", checkout.directory),
        "access": access.map(RepositoryAccess::as_str).unwrap_or("none"),
        "branch": checkout.branch,
        "on_work_branch": checkout.branch == work_branch,
        "head_sha": checkout.head,
        "base_branch": checkout.base,
        "uncommitted_files": checkout.changed_files,
        "commits_ahead_of_base": checkout.ahead_of_base,
        "commits_not_pushed": checkout.unpushed,
        "has_agents_md": checkout.has_instructions,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepoAction {
    Status,
    Add,
    Sync,
    Push,
}

struct RepoTool {
    action: RepoAction,
    config: RepositoriesConfig,
}

impl RepoTool {
    fn new(action: RepoAction, config: &RepositoriesConfig) -> Self {
        Self {
            action,
            config: config.clone(),
        }
    }

    /// Check out every listed repository that is not there yet, then report
    /// all checkouts.
    async fn status(&self, context: &ToolContext) -> Result<Value, ToolExecutionResult> {
        let sandbox = sandbox(context)?;
        let (root, present) = checkouts(sandbox.as_ref(), context).await?;
        let missing: Vec<CloneTarget> = self
            .config
            .repositories
            .iter()
            .filter(|entry| {
                !present.iter().any(|checkout| {
                    checkout
                        .repo
                        .as_deref()
                        .is_some_and(|repo| repo.eq_ignore_ascii_case(&entry.repo))
                })
            })
            .map(|entry| CloneTarget {
                repo: entry.repo.clone(),
                git_ref: entry.git_ref.clone(),
                writable: entry.access == RepositoryAccess::Write,
            })
            .collect();
        let mut failed = Vec::new();
        let mut cloned = Vec::new();
        let present = if missing.is_empty() {
            present
        } else {
            let results =
                clone_repositories(sandbox.as_ref(), context, &self.config, missing).await;
            for (repo, outcome) in results {
                match outcome {
                    Ok((directory, _)) => cloned.push(directory),
                    Err(reason) => failed.push(json!({ "repo": repo, "error": reason })),
                }
            }
            checkouts(sandbox.as_ref(), context).await?.1
        };
        let work_branch = self.config.work_branch(context.session_id);
        let mut repositories = Vec::new();
        for checkout in &present {
            let mut entry = checkout_json(checkout, &self.config, &work_branch);
            if cloned.contains(&checkout.directory)
                && let Some(text) =
                    instructions(sandbox.as_ref(), context, &root, &checkout.directory).await
            {
                entry["agents_md"] = json!(text);
            }
            repositories.push(entry);
        }
        let mut result = json!({
            "work_branch": work_branch,
            "repositories": repositories,
        });
        if !failed.is_empty() {
            result["not_checked_out"] = json!(failed);
        }
        Ok(result)
    }

    async fn add(
        &self,
        arguments: &Value,
        context: &ToolContext,
    ) -> Result<Value, ToolExecutionResult> {
        let repo = required_str(arguments, "repo")?;
        if !is_valid_owner_repo(repo) {
            return Err(ToolExecutionResult::tool_error(
                "repo must be in owner/repo format",
            ));
        }
        let git_ref = match arguments.get("ref").and_then(Value::as_str).map(str::trim) {
            Some(git_ref) if !git_ref.is_empty() => {
                if !is_valid_branch(git_ref) {
                    return Err(ToolExecutionResult::tool_error(
                        "ref is not a valid branch name",
                    ));
                }
                Some(git_ref.to_string())
            }
            _ => self.config.entry(repo).and_then(|e| e.git_ref.clone()),
        };
        let Some(access) = self.config.access(repo) else {
            return Err(ToolExecutionResult::tool_error(format!(
                "{repo} is not one of this session's repositories, and adding others is turned off."
            )));
        };
        let sandbox = sandbox(context)?;
        let (_, present) = checkouts(sandbox.as_ref(), context).await?;
        if present.len() >= MAX_REPOSITORIES
            && !present
                .iter()
                .any(|checkout| checkout.repo.as_deref() == Some(repo))
        {
            return Err(ToolExecutionResult::tool_error(format!(
                "The workspace already holds {MAX_REPOSITORIES} repositories."
            )));
        }
        let target = CloneTarget {
            repo: repo.to_string(),
            git_ref,
            writable: access == RepositoryAccess::Write,
        };
        let mut results =
            clone_repositories(sandbox.as_ref(), context, &self.config, vec![target]).await;
        let (directory, cloned) = results
            .remove(&repo.to_ascii_lowercase())
            .unwrap_or_else(|| Err("clone did not report a result".to_string()))
            .map_err(|reason| {
                ToolExecutionResult::tool_error(format!("Could not check out {repo}: {reason}"))
            })?;
        let (root, present) = checkouts(sandbox.as_ref(), context).await?;
        let work_branch = self.config.work_branch(context.session_id);
        let mut result = present
            .iter()
            .find(|checkout| checkout.directory == directory)
            .map(|checkout| checkout_json(checkout, &self.config, &work_branch))
            .unwrap_or_else(|| json!({ "repo": repo, "path": format!("/workspace/{directory}") }));
        result["already_present"] = json!(!cloned);
        if cloned
            && let Some(text) = instructions(sandbox.as_ref(), context, &root, &directory).await
        {
            result["agents_md"] = json!(text);
        }
        Ok(result)
    }

    /// The checkout of `repo`, as the sandbox reports it.
    async fn checkout_of(
        &self,
        sandbox: &dyn SandboxWorkspace,
        context: &ToolContext,
        repo: &str,
    ) -> Result<Checkout, ToolExecutionResult> {
        let (_, present) = checkouts(sandbox, context).await?;
        present
            .into_iter()
            .find(|checkout| {
                checkout
                    .repo
                    .as_deref()
                    .is_some_and(|found| found.eq_ignore_ascii_case(repo))
            })
            .ok_or_else(|| {
                ToolExecutionResult::tool_error(format!(
                    "{repo} is not checked out in /workspace. Check it out first."
                ))
            })
    }

    async fn sync(
        &self,
        arguments: &Value,
        context: &ToolContext,
    ) -> Result<Value, ToolExecutionResult> {
        let repo = required_str(arguments, "repo")?;
        if !is_valid_owner_repo(repo) || self.config.access(repo).is_none() {
            return Err(ToolExecutionResult::tool_error(format!(
                "{repo} is not one of this session's repositories."
            )));
        }
        let depth = match arguments.get("depth") {
            None => self.config.depth,
            Some(depth) => depth
                .as_u64()
                .filter(|depth| (1..=MAX_DEPTH).contains(depth))
                .ok_or_else(|| {
                    ToolExecutionResult::tool_error(format!(
                        "depth must be an integer from 1 to {MAX_DEPTH}"
                    ))
                })?,
        };
        let sandbox = sandbox(context)?;
        let checkout = self.checkout_of(sandbox.as_ref(), context, repo).await?;
        let branch = match arguments.get("ref").and_then(Value::as_str).map(str::trim) {
            Some(git_ref) if !git_ref.is_empty() => git_ref.to_string(),
            _ => checkout.base.clone(),
        };
        if !is_valid_branch(&branch) {
            return Err(ToolExecutionResult::tool_error(
                "ref is not a valid branch name",
            ));
        }
        let repositories = vec![repo.to_string()];
        let mut access = GitHubAccess::new(self.config.acts_as.into(), &[("contents", "read")]);
        access.repositories = repositories.clone();
        let token = sandbox_token(context, &access, |scope| {
            scope.permits_clone(&repositories, chrono::Utc::now())
        })
        .await?;
        let remote = format!("refs/remotes/origin/{branch}");
        let script = format!(
            "set -eu\n\
             export GIT_TERMINAL_PROMPT=0\n\
             cd -- {directory}\n\
             git -c credential.helper= -c credential.helper={helper} -c core.hooksPath=/dev/null \
             fetch -q --no-tags --depth={depth} {url} {refspec}\n\
             echo \"remote_sha=$(git rev-parse {remote})\"\n\
             echo \"behind=$(git rev-list --count HEAD..{remote} 2>/dev/null || true)\"\n\
             echo \"ahead=$(git rev-list --count {remote}..HEAD 2>/dev/null || true)\"\n",
            directory = quote(&checkout.directory),
            helper = quote(CREDENTIAL_HELPER),
            url = quote(&remote_url(repo)),
            refspec = quote(&format!("+refs/heads/{branch}:{remote}")),
            remote = quote(&remote),
        );
        let response = sandbox
            .exec_with_secret_file(context, &script, sandbox.workspace_root(), &token)
            .await?;
        if response.exit_code != 0 {
            return Err(ToolExecutionResult::tool_error(format!(
                "git fetch failed (exit {}): {}",
                response.exit_code,
                tail(if response.stderr.trim().is_empty() {
                    &response.stdout
                } else {
                    &response.stderr
                })
            )));
        }
        let value = |key: &str| {
            response
                .stdout
                .lines()
                .rev()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .map(str::trim)
                .unwrap_or_default()
                .to_string()
        };
        Ok(json!({
            "repo": repo,
            "path": format!("/workspace/{}", checkout.directory),
            "fetched": format!("origin/{branch}"),
            "remote_sha": value("remote_sha"),
            "commits_behind": value("behind").parse::<u64>().ok(),
            "commits_ahead": value("ahead").parse::<u64>().ok(),
            "hint": format!(
                "Nothing was merged. Bring the changes in with `git merge origin/{branch}` or \
                 `git rebase origin/{branch}`."
            ),
        }))
    }

    async fn push(
        &self,
        arguments: &Value,
        context: &ToolContext,
    ) -> Result<Value, ToolExecutionResult> {
        let repo = required_str(arguments, "repo")?;
        if !is_valid_owner_repo(repo) {
            return Err(ToolExecutionResult::tool_error(
                "repo must be in owner/repo format",
            ));
        }
        // Authorization: the config, by repository name. Nothing the sandbox
        // reports can widen it.
        if self.config.access(repo) != Some(RepositoryAccess::Write) {
            return Err(ToolExecutionResult::tool_error(format!(
                "{repo} is read-only in this session, so nothing can be pushed to it."
            )));
        }
        let sandbox = sandbox(context)?;
        let checkout = self.checkout_of(sandbox.as_ref(), context, repo).await?;
        let work_branch = self.config.work_branch(context.session_id);
        if checkout.branch != work_branch {
            return Err(ToolExecutionResult::tool_error(format!(
                "{repo} is on `{}`. Only the work branch `{work_branch}` is pushed: switch to it \
                 (`git checkout -B {work_branch}`) and try again.",
                checkout.branch
            )));
        }
        if checkout.base == work_branch {
            return Err(ToolExecutionResult::tool_error(
                "The work branch is also the base branch, so there is nothing to compare against.",
            ));
        }
        let access = GitHubAccess::new(self.config.acts_as.into(), &[("contents", "write")])
            .for_repository(repo);
        let token = sandbox_token(context, &access, |scope| {
            scope.permits_push(repo, chrono::Utc::now())
        })
        .await?;
        let remote = format!("refs/remotes/origin/{work_branch}");
        // The URL is built from the authorized name, never read from `origin`.
        // No `--force`: a rewritten work branch is refused by GitHub.
        let script = format!(
            "set -eu\n\
             export GIT_TERMINAL_PROMPT=0\n\
             cd -- {directory}\n\
             git -c credential.helper= -c credential.helper={helper} -c core.hooksPath=/dev/null \
             push -q {url} {refspec}\n\
             git update-ref {remote} HEAD\n\
             echo \"head_sha=$(git rev-parse HEAD)\"\n",
            directory = quote(&checkout.directory),
            helper = quote(CREDENTIAL_HELPER),
            url = quote(&remote_url(repo)),
            refspec = quote(&format!("HEAD:refs/heads/{work_branch}")),
            remote = quote(&remote),
        );
        let response = sandbox
            .exec_with_secret_file(context, &script, sandbox.workspace_root(), &token)
            .await?;
        if response.exit_code != 0 {
            return Err(ToolExecutionResult::tool_error(format!(
                "git push failed (exit {}): {}",
                response.exit_code,
                tail(if response.stderr.trim().is_empty() {
                    &response.stdout
                } else {
                    &response.stderr
                })
            )));
        }
        Ok(json!({
            "repo": repo,
            "branch": work_branch,
            "base_branch": checkout.base,
            "head_sha": checkout.head,
            "uncommitted_files": checkout.changed_files,
            "compare_url": format!(
                "https://github.com/{repo}/compare/{}...{work_branch}",
                checkout.base
            ),
            "hint": "Pushed. Open a pull request from this branch into the base branch when the \
                     change is ready.",
        }))
    }
}

fn repo_property() -> Value {
    json!({ "type": "string", "description": "Repository in owner/repo format." })
}

#[async_trait]
impl Tool for RepoTool {
    fn name(&self) -> &str {
        match self.action {
            RepoAction::Status => "repo_status",
            RepoAction::Add => "repo_add",
            RepoAction::Sync => "repo_sync",
            RepoAction::Push => "repo_push",
        }
    }

    fn display_name(&self) -> Option<&str> {
        Some(match self.action {
            RepoAction::Status => "Repository Status",
            RepoAction::Add => "Add Repository",
            RepoAction::Sync => "Fetch Repository",
            RepoAction::Push => "Push Work Branch",
        })
    }

    fn description(&self) -> &str {
        match self.action {
            RepoAction::Status => {
                "Check out the session's repositories that are not in /workspace yet, then report every repository there: path, access, branch, uncommitted files, commits ahead of the base branch and commits not pushed. Call it first, and again whenever you need the state of the whole change."
            }
            RepoAction::Add => {
                "Check out another GitHub repository at /workspace/<name>, next to the ones already there. Use it when the task turns out to need code from a repository that is not in the session yet."
            }
            RepoAction::Sync => {
                "Fetch a branch of a checked-out repository from GitHub into origin/<branch> (the base branch by default) and report how far the checkout is ahead and behind. It merges nothing. Raise `depth` when you need more history than the checkout has."
            }
            RepoAction::Push => {
                "Push the session's work branch of one writable repository to GitHub. The checkout must be on the work branch. Never forces. Commit first: uncommitted files are not pushed."
            }
        }
    }

    fn parameters_schema(&self) -> Value {
        match self.action {
            RepoAction::Status => json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            RepoAction::Add => json!({
                "type": "object",
                "properties": {
                    "repo": repo_property(),
                    "ref": {
                        "type": "string",
                        "description": "Branch to check out. The default branch when omitted."
                    }
                },
                "required": ["repo"],
                "additionalProperties": false
            }),
            RepoAction::Sync => json!({
                "type": "object",
                "properties": {
                    "repo": repo_property(),
                    "ref": {
                        "type": "string",
                        "description": "Branch to fetch. The repository's base branch when omitted."
                    },
                    "depth": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_DEPTH,
                        "description": "Commits of history to fetch."
                    }
                },
                "required": ["repo"],
                "additionalProperties": false
            }),
            RepoAction::Push => json!({
                "type": "object",
                "properties": { "repo": repo_property() },
                "required": ["repo"],
                "additionalProperties": false
            }),
        }
    }

    fn hints(&self) -> ToolHints {
        let hints = ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true);
        match self.action {
            RepoAction::Status | RepoAction::Add | RepoAction::Sync => hints.with_idempotent(true),
            RepoAction::Push => hints,
        }
    }

    fn narrate(
        &self,
        call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_contracts::runtime::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_contracts::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let (labels, labels_uk) = match self.action {
            RepoAction::Status => (
                (
                    "Checking repositories",
                    "Checked repositories",
                    "Could not check repositories",
                ),
                (
                    "Перевіряю репозиторії",
                    "Перевірив репозиторії",
                    "Не вдалося перевірити репозиторії",
                ),
            ),
            RepoAction::Add => (
                ("Checking out", "Checked out", "Could not check out"),
                ("Завантажую", "Завантажив", "Не вдалося завантажити"),
            ),
            RepoAction::Sync => (
                ("Fetching", "Fetched", "Could not fetch"),
                (
                    "Отримую зміни",
                    "Отримав зміни",
                    "Не вдалося отримати зміни",
                ),
            ),
            RepoAction::Push => (
                ("Pushing", "Pushed", "Could not push"),
                ("Надсилаю", "Надіслав", "Не вдалося надіслати"),
            ),
        };
        Some(
            everruns_contracts::runtime::tool_narration::narrate_labeled_action(
                &call.arguments,
                phase,
                locale,
                labels,
                labels_uk,
                &["repo"],
            ),
        )
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(format!("{} requires session context.", self.name()))
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let result = match self.action {
            RepoAction::Status => self.status(context).await,
            RepoAction::Add => self.add(&arguments, context).await,
            RepoAction::Sync => self.sync(&arguments, context).await,
            RepoAction::Push => self.push(&arguments, context).await,
        };
        match result {
            Ok(value) => ToolExecutionResult::success(value),
            Err(error) => error,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[path = "repositories_tests.rs"]
mod tests;
