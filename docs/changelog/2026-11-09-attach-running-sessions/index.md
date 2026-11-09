---
title: "Attach a second terminal to a running Framework session"
description: "Framework hosts serve live sessions on a private local socket, so another process of the same user can list, watch, send to and cancel them."
date: 2026-11-09
kind: feature
significance: notable
products: [framework]
surfaces: [code]
links:
  docs: [/framework/attach-control/]
---

A Framework host can now let a second process drive its live sessions. With
the `control` feature, the host registers sessions in a `ControlRegistry` and
starts a `ControlServer`; another terminal run by the same user lists them,
streams their output, sends input and cancels the running turn with a
`ControlClient`.

```rust
// everruns = { features = ["control", "openai"] }
use everruns::control::{ControlClient, default_dir};

async fn stop_it() -> Result<(), everruns::control::ControlClientError> {
    let client = ControlClient::new(default_dir("myagent"));
    client.cancel("main").await?;
    Ok(())
}
```

Access is limited to the same OS user and a token that rotates and can be
revoked. See [Attach to a Running Session](/framework/attach-control/) and the
`attached-cli` example.
