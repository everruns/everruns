---
title: Stored attachments download safely in browsers
description: Stored files now download as inert binary attachments while agents retain their original bytes and media types across Slack, AG-UI and A2A.
date: 2026-10-13
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [api, agent]
links:
  docs: [/features/channels/]
---

Stored attachments now download when you open their file URL, including HTML,
script files, PDFs and zip archives. The download endpoint returns an inert
binary media type and retains its filename and security headers, so received
files cannot become active documents on the application's origin.

Agents still receive the same bytes and media metadata, and file listings keep
the original media type. Authentication and organization membership are still
required. See [Channels](/features/channels/#download-received-files) for the
download policy.
