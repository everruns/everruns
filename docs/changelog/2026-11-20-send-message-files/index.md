---
title: Agents send files with their messages
description: An explicit agent attaches the chart, PDF or report it made to send_message, and it lands under its message in Slack and in web chat.
date: 2026-11-20
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [agent, ui]
links:
  docs: [/features/explicit-communication/, /capabilities/slack/]
---

An agent that renders a chart no longer posts "here it is" and then makes a
second call to upload it. With [explicit communication](/features/explicit-communication/),
`send_message` takes `files`: session paths delivered with the message, up to
10 files and 8 MiB combined. In [Slack](/capabilities/slack/) they land in the
same thread right under the text, as the channel's bot. In web chat they show
under the message, ready to download.

```json
{
  "text": "Signups doubled after the launch. Chart and raw numbers attached.",
  "files": ["/out/signups.png", "/out/signups.csv"]
}
```

The agent never names a channel or thread, for messages or for files.
`slack_upload_file` stays for agents that reply directly, and it no longer asks
for `channel` or `thread_ts` either: Everruns uses the session's own thread.
