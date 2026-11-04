---
title: Framework and serve keep files in the session
description: Session::attach keeps a file in the session's /attachments folder, serve's message endpoint takes a files list, and embedded Slack files are kept too.
date: 2026-10-14
kind: feature
significance: minor
products: [framework]
release: "0.49.0"
surfaces: [code, api]
links:
  docs: [/framework/sessions/#send-files, /framework/serve/#wire-api]
  prs: [everruns/everruns#4720]
---

Files sent to a Framework or serve agent now stay readable for the whole conversation.
`Session::attach` writes a file read-only to `/attachments/<id>/<name>` in the session's own files
and returns the part that sends it. The agent can open it again with `read_file` after compaction,
and a zip unpacks into a folder next to it.

```rust,no_run
# async fn demo(session: everruns::Session) -> Result<(), Box<dyn std::error::Error>> {
# use everruns::InputMessage;
let report = session
    .attach("report.pdf", Some("application/pdf"), std::fs::read("report.pdf")?)
    .await?;
let mut message = InputMessage::user("Summarize the attached report.");
message.content.push(report);
session.run(message).await?;
# Ok(())
# }
```

A serve app's message endpoint takes the same `files` list as the Agent API, and embedded Slack
files are kept the same way. See [Send files](/framework/sessions/#send-files) and the
[serve wire API](/framework/serve/#wire-api).
