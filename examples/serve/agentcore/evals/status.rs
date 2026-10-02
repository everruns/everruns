use serve::prelude::*;

/// Asking about an order looks it up.
#[eval]
async fn looks_up_the_order(t: &mut EvalCx) -> Result {
    t.send("Where is order A-1001?").await?;
    t.completed()?
        .called_tool("order_status")?
        .reply_contains("shipped")?;
    Ok(())
}
