---
title: Slack agents on your own computer, no public address
description: A Slack channel can now use Socket Mode, where Everruns connects out to Slack, so a self-hosted agent works in Slack without a tunnel or public HTTPS.
date: 2026-11-08
kind: feature
significance: minor
products: [platform]
surfaces: [ui, api]
links:
  docs: [/capabilities/slack/#socket-mode-or-request-url]
---

A Slack channel now chooses [how Slack reaches Everruns](/capabilities/slack/#socket-mode-or-request-url).
With **Socket Mode**, Everruns opens a connection out to Slack and receives
messages over it, so an instance on a laptop, behind a home router or a
firewall, answers in Slack with no tunnel and no `PUBLIC_APP_URL`.

New channels pick Socket Mode when the Everruns page is not on public HTTPS.
Create the app from the channel's manifest, generate an app-level token with
`connections:write` in Slack, and paste it beside the bot token. The channel
shows **Connected** once Slack accepts the connection, and reconnects on its own
after a restart. Over the API it is two fields of the channel config:

```json
{
  "connection": "socket_mode",
  "app_token": "xapp-1-...",
  "bot_token": "xoxb-..."
}
```

**Request URL** stays the default on a public server, and existing channels keep it.
