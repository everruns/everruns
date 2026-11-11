---
title: "Slack replies render Markdown everywhere, and never ping @here"
description: "Framework Slack replies and file comments now render Markdown, broadcast mentions post as plain text, and a reply Slack rejects still arrives as text."
date: 2026-11-11
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [api, code]
links:
  docs: [/capabilities/slack/]
---

Agents write standard Markdown, and every Slack post now carries it as a
Slack `markdown` block. Before this, the Framework's Slack driver and the
comment posted with `slack_upload_file` sent plain `text`, which Slack reads
as its own `mrkdwn` dialect: `**bold**`, `## headings` and Markdown links
showed up as raw characters.

`<!here>`, `<!channel>` and `<!everyone>` in agent output now post as plain
`@here` text, so a confused or prompt-injected agent cannot notify a whole
channel. When Slack rejects a reply's blocks, for example over a mention of a
user group that does not exist, the reply is sent again as plain text rather
than lost. Agents with Slack actions also learn what Slack renders differently:
mentions as `<@USERID>`, images only as links, no HTML. See
[Slack formatting](/capabilities/slack/).

With the Framework's `channels` feature, the driver needs no change:

```rust
use everruns::channels::{Channel, SessionBinding, Slack};

fn main() {
    // Replies to this channel post as Slack `markdown` blocks.
    let _slack = Channel::new(Slack::new("xoxb-your-bot-token", "signing-secret"))
        .binding(SessionBinding::Thread);
}
```
