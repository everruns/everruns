---
title: "Attachments stay readable after compaction, from every channel"
description: "Files sent to an agent are kept in the session's /attachments folder, so the agent can reopen them after compaction, whichever channel they came from."
date: 2026-10-13
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api, agent]
links:
  docs: [/features/channels/, /how-to/call-your-agent-from-code/]
  prs: [everruns/everruns#4636, everruns/everruns#4650, everruns/everruns#4654, everruns/everruns#4661, everruns/everruns#4677, everruns/everruns#4686]
---

Files sent to an agent no longer disappear from its view when a long conversation is compacted.
Each file is copied into the session's `/attachments` folder, and the model sees every file by
name and path, so it can open one again with `read_file`. PDFs reopen as documents, and a zip
unpacks to a read-only folder next to the archive.

Files now arrive the same way from Chat, Slack, AG-UI, Public Chat, A2A, the Agent API and the
MCP tools. A file that cannot be taken becomes a note the agent reads:

```text
[Attachment report.pdf not received: 31 MB is over the 25 MB limit]
```

See [Files people send](/features/channels/#files-people-send), and
[Send files](/how-to/call-your-agent-from-code/#send-files) for the Agent API.
