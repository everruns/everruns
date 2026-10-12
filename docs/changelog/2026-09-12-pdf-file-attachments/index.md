---
title: Attach PDF files to agent messages
description: Messages can now carry PDF attachments, uploaded through a new files API and passed to each provider in its native document format.
date: 2026-09-12
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.26.0"
links:
  docs: [/features/ui/]
  prs: [everruns/everruns#3535]
visual: "Chat composer with a PDF file chip attached before sending (/chats/<id>); crop to the composer"
---

Agents can read PDF documents sent with a message.

**In the UI:** the chat composer offers PDF attachments when the selected model
accepts PDF input, and shows each attachment as a file chip.

**In the API:** upload a PDF with `POST /v1/files` (PDF only, up to 32 MB), then
reference it as a `file` content part in the message. `GET /v1/files`,
`GET /v1/files/{id}` and `DELETE` manage stored files. Everruns converts the
attachment for each provider, including Anthropic documents, Gemini inline data,
Bedrock documents and OpenAI file inputs.

See [UI](/features/ui/). [#3535](https://github.com/everruns/everruns/pull/3535)
