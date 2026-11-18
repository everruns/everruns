---
title: GitHub tools
description: Read pull requests and repository files, search GitHub, and explicitly enable write tools in a Framework application.
---

Give an agent GitHub tools with `everruns::github::GitHub`. The capability
uses the same GitHub operations as the Platform, with credentials your
application supplies. It requires no Everruns server or database.

Until these APIs are included in the next crate release, use the repository’s
`main` branch or run the examples from a source checkout:

```toml
[dependencies]
everruns = { git = "https://github.com/everruns/everruns", branch = "main", features = ["github", "openai"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The `github` feature is optional. It adds the GitHub HTTP integration without
enabling other vendors or the hosted capability catalog.

## Read a pull request

Set `GITHUB_TOKEN` with repository read access and `OPENAI_API_KEY`, then run:

```rust
use everruns::{Agent, Engine, OpenAI, github::GitHub};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Read the requested pull request and its diff. Report concrete bugs with evidence.")
        .provider(OpenAI::from_env()?)
        .model("gpt-6.2-sol")
        .capability(GitHub::from_env()?)
        .max_iterations(12)
        .build()?;
    let engine = Engine::new();
    let result = engine.create(agent)
        .send_and_wait("Review pull request 123 in my-org/my-repo.")
        .await?;
    println!("{}", result.response);
    Ok(())
}
```

The [complete PR reviewer example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/github_review.rs)
takes the repository and PR number as arguments:

```bash
cargo run -p everruns --features github,openai --example github_review -- OWNER/REPO NUMBER
```

It runs a real model against GitHub and prints its review. Its GitHub capability
is read-only, so it cannot post comments or reviews.

## Tool selection and write controls

| Configuration | Available tools |
|---|---|
| Default | `get_github_pull_request`, `get_github_pull_request_diff`, `search_github_code`, `read_github_file`, `search_github_issues` |
| `.allow_writes(true)` | Also `upsert_github_comment`, `submit_github_pull_request_review`, `upsert_github_issue` |
| `.allow_writes(true).allow_pull_requests(true)` | Also `create_github_pull_request` |
| `.allow_writes(true).allow_merge(true)` | Also `mark_github_pull_request_ready`, `merge_github_pull_request` |

Enable writes only for an application that should modify GitHub:

```rust
use everruns::github::GitHub;

let github = GitHub::from_env()?
    .allow_writes(true)
    .private_issues_only(true)
    .allow_pull_requests(true);
# Ok::<(), Box<dyn std::error::Error>>(())
```

These options decide which tools exist; instructions cannot enable an absent
tool. The GitHub token also needs the corresponding repository permissions.
`private_issues_only(true)` checks repository visibility before filing an issue.
Reviews can comment or request changes, and cannot approve. PR creation defaults
to a draft and returns an existing open PR from the same head branch.

Comments and findings carry markers for updates and duplicate suppression.
Finding/review matching trusts bot-authored records. Use a GitHub App installation
token for repeatable bot writes; personal-token runs do not gain a bot identity.

## Tokens and refresh

`GitHub::new(token)` keeps a token in application memory. `GitHub::from_env()`
reads `GITHUB_TOKEN` once at startup and rejects empty values. Neither stores
credentials in capability configuration or includes them in debug output.

For short-lived installation tokens or rotating credentials, implement
`GitHubTokenProvider`. It is called on each operation with the repository and
requested permissions. An application can mint the least-privileged token it
needs; a static personal token retains its original permissions.

```rust
use everruns::capability::async_trait;
use everruns::github::{GitHub, GitHubTokenProvider, GitHubTokenRequest};

struct ApplicationTokens;

#[async_trait]
impl GitHubTokenProvider for ApplicationTokens {
    async fn token(&self, request: &GitHubTokenRequest) -> Result<String, String> {
        // Replace this with your application's refresh or installation-token service.
        // request.repository and request.permissions describe this operation's needs.
        let _ = request;
        std::env::var("GITHUB_TOKEN").map_err(|error| error.to_string())
    }
}

let github = GitHub::with_token_provider(ApplicationTokens);
```

Credential-provider failures are returned as generic errors; raw provider errors
and GitHub error response bodies are not exposed to the model. A failed provider
does not fall back to process or session credentials.

`.api_base("https://github.example.com/api/v3")` selects a trusted GitHub Enterprise
API root. It receives the credential, so configure it in application code.
Search availability follows your GitHub installation and token permissions.

## Platform boundary

The Framework adapter shares tool schemas, validation, diff limits, review
policy, and vendor operations with the Platform. The Platform still owns GitHub
App setup, webhooks/triggers, user-versus-agent identity selection, and connection
storage. Managed sandbox checkout requires an attested installation credential
and a sandbox service; this Framework capability does not advertise that tool.

See [capability integrations](/framework/capability-integrations/) for other
optional integrations and [moving to Platform](/framework/moving-to-platform/)
for deployment differences.
