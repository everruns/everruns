use serve::prelude::*;

/// A revenue question is answered from the warehouse, net of refunds.
#[eval]
async fn answers_net_of_refunds(t: &mut EvalCx) -> Result {
    t.send("What was revenue last week?").await?;
    t.completed()?
        .called_tool("run_sql")?
        .reply_contains("net of refunds")?;
    Ok(())
}

/// A query that scans every order waits for a person first.
#[eval]
async fn full_scans_need_approval(t: &mut EvalCx) -> Result {
    t.send("What was revenue last week?").await?;
    t.send("Now show me every order we have ever had.").await?;
    t.completed()?.asked_approval("run_sql")?;
    Ok(())
}
