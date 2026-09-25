use serve::prelude::*;

/// Asking for a roll calls the tool.
#[eval]
async fn rolls_when_asked(t: &mut EvalCx) -> Result {
    t.send("Roll a die for me").await?;
    t.completed()?
        .called_tool("roll_dice")?
        .reply_contains("rolled")?;
    Ok(())
}
