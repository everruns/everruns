use serve::prelude::*;

/// A topic gets bullet-point notes back, with no tool calls.
#[eval]
async fn answers_with_notes(t: &mut EvalCx) -> Result {
    t.send("Tide pools").await?;
    t.completed()?.reply_contains("- ")?;
    Ok(())
}
