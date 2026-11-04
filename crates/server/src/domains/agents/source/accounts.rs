//! Development GitHub accounts a source can read through, for the Sync dialog.
//!
//! Decision: the dialog offers the person's own sign-in and the organization's
//! shared App links side by side, with the repositories each reaches, so a
//! person connects GitHub inline instead of setting the Agent's runtime identity.
use super::*;

/// One repository a development account can reach.
#[derive(Debug, Serialize, ToSchema)]
pub struct SourceRepository {
    /// Repository in owner/name form.
    #[schema(example = "acme/agents")]
    pub full_name: String,
    /// GitHub account that owns the repository.
    #[schema(example = "acme")]
    pub account: String,
    pub private: bool,
}

/// The caller's own GitHub sign-in.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonalSourceAccount {
    /// The caller has signed in to GitHub.
    pub connected: bool,
    /// GitHub login of that sign-in.
    #[schema(example = "octocat")]
    pub login: Option<String>,
    /// Repositories reachable through the shared App's installations the person can access.
    pub repositories: Vec<SourceRepository>,
    /// Why repositories could not be listed, when they could not.
    pub error: Option<String>,
}

/// The organization's links to the shared Everruns GitHub App.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrganizationSourceAccount {
    /// The caller may link another GitHub account (organization admins).
    pub can_connect: bool,
    /// Linked GitHub accounts.
    pub accounts: Vec<String>,
    /// Repositories reachable through the linked installations.
    pub repositories: Vec<SourceRepository>,
    /// Why repositories could not be listed, when they could not.
    pub error: Option<String>,
}

/// Development GitHub accounts available to an Agent's source sync.
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentSourceAccounts {
    /// The deployment has the shared Everruns GitHub App configured.
    pub available: bool,
    /// Where a person installs the shared Everruns App on another GitHub account.
    pub install_url: Option<String>,
    pub personal: PersonalSourceAccount,
    pub organization: OrganizationSourceAccount,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListAgentSourceAccounts {
    pub agent_id: String,
}
#[command(name="list_agent_source_accounts",summary="List GitHub accounts for Agent source sync",category="agents",description="Show the caller's own GitHub sign-in and the organization's linked GitHub accounts that source sync can read through, with their repositories. Never the Agent's runtime GitHub identity.",method="GET",path="/v1/agents/{agent_id}/source/accounts",policy=AGENT_VIEW,http=plain,cli=CliRoute::new(&["agents", "source", "accounts"], "list").with_examples(&[CliExample::new("See which GitHub accounts and repositories can back this Agent's source", "everruns agents source accounts list --agent-id agent_01h9")]))]
impl Command for ListAgentSourceAccounts {
    type Output = AgentSourceAccounts;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        agent(ctx, &self.agent_id).await?;
        let personal = personal(ctx).await?;
        let organization = organization(ctx).await?;
        Ok(AgentSourceAccounts {
            available: ctx.github_apps.shared_app_slug.is_some(),
            install_url: ctx.github_apps.shared_app_slug.as_deref().map(|slug| {
                format!(
                    "{}/apps/{slug}/installations/new",
                    ctx.github_apps
                        .api
                        .endpoints()
                        .web_url
                        .trim_end_matches('/')
                )
            }),
            personal,
            organization,
        })
    }
}

fn unlimited() -> everruns_contracts::runtime::GitHubAccess {
    everruns_contracts::runtime::GitHubAccess::new(everruns_core::McpServerActsAs::Service, &[])
}

async fn connection_token(ctx: &Ctx, access: &SourceAccess) -> Result<String, CommandError> {
    ctx.connection_resolver
        .as_ref()
        .ok_or_else(|| CommandError::unavailable("GitHub connection resolver is unavailable"))?
        .get_github_token_for_connection(
            ctx.org_id(),
            access.identity,
            access.connection,
            &unlimited(),
        )
        .await
        .ok()
        .flatten()
        .ok_or_else(|| CommandError::bad_request("GitHub access expired. Connect GitHub again"))
}

fn repositories(value: &Value) -> Vec<SourceRepository> {
    value["repositories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|repo| {
            let full_name = repo["full_name"].as_str()?.to_string();
            Some(SourceRepository {
                account: full_name.split_once('/')?.0.to_string(),
                private: repo["private"].as_bool().unwrap_or(false),
                full_name,
            })
        })
        .collect()
}

async fn get(ctx: &Ctx, path: &str, token: &str) -> Result<Value, CommandError> {
    ctx.github_apps
        .api
        .read_source_json(path, token)
        .await
        .map_err(|e| CommandError::bad_request(e.to_string()))
}

async fn personal(ctx: &Ctx) -> Result<PersonalSourceAccount, CommandError> {
    let Ok(access) = super::personal_access(ctx).await else {
        return Ok(PersonalSourceAccount {
            connected: false,
            login: None,
            repositories: Vec::new(),
            error: None,
        });
    };
    let login = ctx
        .db
        .get_virtual_user_connection_by_id(
            VirtualUserId::from_uuid(access.identity),
            access.connection,
            "github",
        )
        .await?
        .and_then(|c| c.provider_username);
    let listed = async {
        let token = connection_token(ctx, &access).await?;
        // A user token reaches the shared App's installations the person can access.
        let installations = get(ctx, "/user/installations?per_page=100", &token).await?;
        let mut found = Vec::new();
        for id in installations["installations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["id"].as_i64())
            .take(20)
        {
            let page = get(
                ctx,
                &format!("/user/installations/{id}/repositories?per_page=100"),
                &token,
            )
            .await?;
            found.extend(repositories(&page));
        }
        Ok::<_, CommandError>(found)
    }
    .await;
    Ok(finish_personal(login, listed))
}

fn finish_personal(
    login: Option<String>,
    listed: Result<Vec<SourceRepository>, CommandError>,
) -> PersonalSourceAccount {
    let (mut repositories, error) = match listed {
        Ok(found) => (found, None),
        Err(e) => (Vec::new(), Some(e.message().to_string())),
    };
    repositories.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    repositories.dedup_by(|a, b| a.full_name == b.full_name);
    PersonalSourceAccount {
        connected: true,
        login,
        repositories,
        error,
    }
}

async fn organization(ctx: &Ctx) -> Result<OrganizationSourceAccount, CommandError> {
    let links = ctx.db.list_everruns_app_links(ctx.org_id()).await?;
    let mut accounts = Vec::new();
    let mut found = Vec::new();
    let mut error = None;
    for link in links {
        let Some(login) = link.provider_username.clone() else {
            continue;
        };
        accounts.push(login);
        let access = SourceAccess {
            account: SourceAccountKind::Organization,
            identity: link.virtual_user_id.uuid(),
            connection: link.id,
            installation: link.installation_id,
        };
        let listed = async {
            let token = connection_token(ctx, &access).await?;
            get(ctx, "/installation/repositories?per_page=100", &token).await
        }
        .await;
        match listed {
            Ok(page) => found.extend(repositories(&page)),
            Err(e) => error = Some(e.message().to_string()),
        }
    }
    found.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    Ok(OrganizationSourceAccount {
        can_connect: ctx
            .caller
            .role
            .has_permission(everruns_core::OrgRole::Admin),
        accounts,
        repositories: found,
        error,
    })
}
