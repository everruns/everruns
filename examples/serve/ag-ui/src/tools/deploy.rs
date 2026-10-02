use serve::prelude::*;

#[derive(Serialize)]
struct Deployment {
    environment: String,
    status: &'static str,
}

/// Deploy the current build to an environment.
///
/// Needs approval: an AG-UI run ends with a `tool_approval` interrupt, and
/// the client's next run resumes it with `{ "decision": "allow" }` or
/// `{ "decision": "reject" }`.
#[tool(needs_approval)]
async fn deploy(
    cx: &Cx,
    /// The environment to deploy to, such as `staging` or `production`.
    environment: String,
) -> Result<Deployment> {
    cx.progress(format!("deploying to {environment}")).await;
    Ok(Deployment {
        environment,
        status: "deployed",
    })
}
