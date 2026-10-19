use serve::prelude::*;

/// A question about a night checks availability before answering. A call
/// reaches the agent as the same kind of message, so this covers voice too.
#[eval]
async fn checks_rooms_before_answering(t: &mut EvalCx) -> Result {
    t.send("Do you have a room on Friday?").await?;
    t.completed()?
        .called_tool("rooms")?
        .reply_contains("Friday")?;
    Ok(())
}
