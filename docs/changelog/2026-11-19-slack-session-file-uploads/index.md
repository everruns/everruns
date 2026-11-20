---
title: Slack agents can share session files
description: Slack agents can upload existing PNG, PDF, MP4, and other session files to the originating thread using the same bot identity.
date: 2026-11-19
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/slack/]
---

A Slack agent can upload a file from its session with `slack_upload_file`.
The file is delivered to the originating thread as the channel’s bot. The
tool still accepts inline text for reports and logs.

The bot needs the `files:write` scope. See the [Slack capability guide](/capabilities/slack/).

```json
{
  "channel": "C0123456789",
  "thread_ts": "1791063223.531299",
  "filename": "promo.png",
  "session_path": "/out/promo.png"
}
```
