use serve::prelude::*;

/// Asking for a deployment asks for approval, then deploys (evals approve
/// by default).
#[eval]
async fn deploys_after_approval(t: &mut EvalCx) -> Result {
    t.send("Deploy to staging").await?;
    t.completed()?
        .asked_approval("deploy")?
        .called_tool("deploy")?
        .reply_contains("Deployed")?;
    Ok(())
}
