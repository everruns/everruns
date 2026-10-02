use serve::prelude::*;

/// The agent works in the shell, then asks before sharing.
#[eval]
async fn works_in_the_shell_then_shares_after_approval(t: &mut EvalCx) -> Result {
    t.send("Add 'check the build' to my todo list and share it")
        .await?;
    t.completed()?
        .called_tool("bash")?
        .asked_approval("share_report")?
        .called_tool("share_report")?
        .reply_contains("notes/todo.md")?;
    Ok(())
}
