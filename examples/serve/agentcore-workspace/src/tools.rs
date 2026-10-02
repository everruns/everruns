use serve::prelude::*;

#[derive(Serialize)]
struct Shared {
    title: String,
    status: &'static str,
}

/// Share a report outside the workspace.
///
/// Needs approval: over AG-UI the run ends with a `tool_approval` interrupt,
/// and the client's next run answers it.
#[tool(needs_approval)]
async fn share_report(
    cx: &Cx,
    /// A short title.
    title: String,
    /// The report, in Markdown.
    body: String,
) -> Result<Shared> {
    cx.progress(format!("sharing {title} ({} bytes)", body.len()))
        .await;
    // A real app posts to Slack, email or a ticket here.
    Ok(Shared {
        title,
        status: "shared",
    })
}
