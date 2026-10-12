---
type: Decision
title: "Session Attachments"
description: "How files people send to an agent stay usable for the whole session on every surface: one intake path that stores the bytes in session file storage, a label with a stable /attachments path on every attachment, a deterministic attachment list after compaction, and one-at-a-time file resolution."
tags:
  - everruns
  - runtime-resources
  - attachments
  - compaction
  - channels
---
# Session Attachments

> Status: **Accepted 2026-10-12, in progress.** It changes how
> [Compaction](compaction.md), [Channels](../integrations/channels.md), the
> [Session Filesystem](file-store.md) and [Object Storage](object-storage.md)
> treat files that people attach to messages.

## Abstract

An attachment is a file a person (or a calling system) sends along with a
message: a screenshot in chat, a PDF in Slack, a `FilePart` over A2A. Today an
attachment is only a part of one message. The model sees it as long as that
message is in the request, and nothing else in the session knows it exists.

This design makes an attachment a session resource. Every surface stores the
bytes in the session file storage through one intake path and puts a reference
into the message. The model always sees a short label that names a stable
path, such as `/attachments/a1b2c3/report.pdf`. The bytes follow the label as
they do today; when compaction or a model without that modality leaves them
out, the label stays and the agent can open the path again with `read_file`
or the shell.

## Is it actually a problem?

Partly. Session storage never loses an attachment, and in a short chat session
the model sees every image and PDF on every turn. The losses are in four other
places, and the biggest one is not compaction but intake.

### What holds today

| Case | Verdict | Where |
|---|---|---|
| Stored history | Lossless. `image_file` / `file` parts are stored as references, inline `image` parts as base64 in the event. | [message parts](../../crates/contracts/src/runtime/message.rs) |
| Replay on later turns | Every image and PDF still in the window is resolved and re-sent on every turn. | [`resolve_images` / `resolve_files`](../../crates/core/src/engine/execution/reason.rs) |
| Fork | Preserved: events are copied and IDs stay org-scoped. | [fork](../../crates/server/src/domains/sessions/service/fork.rs) |
| Summarization is per request | The summary is not written as a durable checkpoint, so the next turn starts from full history again. | [reactive compaction](../../crates/core/src/engine/execution/reason/compaction.rs) |
| Cost-control masking | Keeps tool-result images (rewrites only the text). | [compaction builtins](../../crates/core/src/builtins/compaction.rs) |

### Where attachments are lost

**Compaction and context management.** When a request is compacted, the model
loses older attachments with no trace and no way back:

- LLM summarization feeds the summarizer text parts only (`extract_text` in
  [compaction builtins](../../crates/core/src/builtins/compaction.rs)). Images
  and PDFs older than the last 10 messages disappear without a marker, a file
  name or an ID, and the summary prompt never mentions them.
- Native provider compaction folds them into an opaque blob; no name survives.
- Aggressive trim drops whole messages (only the first one is anchored).
- Observation masking replaces old tool results, including screenshots and
  `read_file` images, with their text only.
- Infinity context evicts messages, and its `query_history` tool returns text
  and tool-result text only
  ([infinity context](../../crates/core/src/builtins/infinity_context.rs)).
- Nothing the agent can call takes an `image_id` or `file_id`, and uploads
  never land in the session filesystem, so even an agent that remembers "the
  user sent a contract" cannot look at it again.

**Intake on most surfaces.** Outside the web chat and the raw REST API, files
mostly never become attachments at all:

| Surface | Today | Where |
|---|---|---|
| Web chat | Images (10) and PDFs (5) upload, then travel as references. PDF only; the UI allows 32 MB but the server caps at 11 MiB. | [`use-file-attachments`](../../apps/ui/src/hooks/use-file-attachments.ts), [`files.rs`](../../crates/server/src/api/files.rs) |
| REST messages | Any input part. Inline base64 images are stored in the plaintext `events.data` column and its search vector. | [`messages.rs`](../../crates/server/src/api/messages.rs) |
| Slack (server) | Images forward Slack's `url_private`, which needs the bot token, straight to the provider (nothing downloads it). Every other file becomes `[Attached file: name (mime)]`. Thread backfill ignores files. | [`inbound.rs`](../../crates/server/src/channels/slack/events/inbound.rs), [`host.rs`](../../crates/core/src/channel_runtime/host.rs) |
| Slack (embedded integration) | Every file, images included, becomes name and type text. | [`slack_channel`](../../crates/integrations/src/slack_channel/mod.rs) |
| AG-UI, Public Chat | The trigger message is flattened with `to_text()`; inline image, audio, document parts are dropped silently. Only a separate image upload route plus `forwardedProps.imageIds` works. | [`ag_ui/mod.rs`](../../crates/server/src/channels/ag_ui/mod.rs) |
| A2A | Only text parts are read; `FilePart` (bytes or URI) is dropped silently. PACT rejects non-text parts. | [`a2a/mod.rs`](../../crates/server/src/channels/a2a/mod.rs), [`pact.rs`](../../crates/server/src/channels/a2a/pact.rs) |
| Agent execution API, MCP `session_send_message`, webhook, Poppy, voice, CLI | Text only. | [`api_sessions.rs`](../../crates/server/src/domains/agent_channels/api_sessions.rs) |

**Request limits.** Resolution is all or nothing:

- Files resolve in one gRPC batch capped at 8 files and 11 MiB combined. A
  session with a ninth PDF, or three 4 MB PDFs, fails the batch and **every**
  PDF in the window becomes `[File not found: id]`
  ([`artifacts.rs`](../../crates/server/src/worker_link/grpc_service/worker/artifacts.rs)).
  The in-process worker has no such cap, so the two hosts behave differently.
- Images have no per-request count, no size budget and no downscaling, so a
  long screenshot-heavy session can hit a provider's image limit.
- A model switch to a model without vision or PDF input re-sends every part
  as is; model modality flags are catalog metadata only.
- Compaction's token estimate counts every non-text part as about 12 tokens,
  so image-heavy contexts trigger proactive compaction late.

**Delegation.** A subagent receives its instructions as one text message
(`PlatformStore::send_message(session_id, &str)` in
[subagents](../../crates/capabilities/src/capabilities/subagents.rs)), so a
parent cannot hand a child the PDF it was asked about.

So the person's suspicion holds for short chat sessions, which work. The
reported loss is real after compaction, and the larger gap is that Slack,
AG-UI, A2A and the API surfaces mostly never deliver the file in the first
place.

## Goals

1. A file sent on any surface reaches the agent as an attachment, or as a note
   that says why it did not. Nothing is dropped silently.
2. The agent can always tell which attachments exist in the conversation and
   open any of them again, whatever compaction did to the request.
3. No single attachment can make the others disappear from a request.
4. One mechanism for server, worker, Framework and serve hosts.

Non-goals: outbound files from the agent to a surface, text extraction for
office formats, audio or video understanding, files that tools produce, and a
per-request byte budget (bytes keep travelling with their message until
compaction removes it, as today).

## Design

The design stays small on purpose: no new store, no new tool, no mount. It
reuses the session file storage every session already has.

### 1. Attachments live in session file storage

Intake writes each attachment's bytes into the session's file storage at
`/attachments/<short-id>/<filename>`, and the message keeps a reference part
that names that path, the file name, the verified media type and the size.

- **Every session has file storage.** A server session always has a
  Workspace (database rows with optional S3 offload, see
  [Object Storage](object-storage.md)); Framework and serve hosts always have a
  `SessionFileSystem` (in-memory by default, real disk or a custom store when
  configured). Forks already copy it, and it is deleted with the session, so
  attachments need no garbage collection of their own.
- **Only Everruns writes `/attachments`.** It joins the runtime artifact roots
  (`/.agent-runs`, `/.tasks`, `/.background`) in
  [`WorkspacePolicy`](../../crates/core/src/workspace_policy.rs): the model can
  read it, never write or delete it, so a label can never point at bytes the
  agent changed. Copies are also stored read-only, so the Workspace API
  cannot overwrite them either. They are ordinary session files otherwise:
  they show in the file browser, count toward the Workspace quota, and are
  shared when sessions share a Workspace. A copy that fails (quota, storage
  error) is logged and skipped; the model still gets the bytes that turn, it
  just cannot reopen them later. An agent that wants to edit a file copies it
  elsewhere first.
- **Uploads before a session exists.** The images and files APIs keep
  staging chat uploads. Message creation copies each referenced upload into
  `/attachments` (after checking it belongs to the org), so uploads get a path
  like every other attachment.
- **Reopening.** `read_file` already shows PNG, JPEG, GIF and WebP to the
  model as images, and PDFs (sniffed by their `%PDF-` header) as documents.
  Tool results carry both in the same inline-media list; the one conversion
  into LLM parts turns a non-image entry into a file part, and each driver
  sends it natively inside or right after the tool result (Anthropic
  `document` block, Bedrock document block, Gemini inline data after the
  function response, a `user` carrier on chat completions, `input_file` on
  Responses). The per-result 5 MiB inline-media cap applies, so a larger PDF
  is reported as dropped. No model capability check gates this yet: a model
  that cannot read PDFs gets the provider's error. The shell sees
  `/attachments` like any other path. Giving an agent without file tools a
  `read_file` limited to `/attachments` is left until a surface needs it.

### 1b. Zip attachments open as folders

A zip attachment is unpacked next to itself inside `/attachments`, read-only:
`/attachments/<id>/data.zip` gets `/attachments/<id>/data/`. Its label names
the folder (`..., unpacked to /attachments/<id>/data/`) and the zip bytes are
never sent to the model, so the agent browses the files with `read_file` or
the shell.

- **Bomb limits** in
  [`zip_attachments.rs`](../../crates/server/src/domains/messages/zip_attachments.rs):
  1,000 files, 100 MiB unpacked, 25 MiB per file, and a 100:1 compression
  ratio, all measured while reading rather than trusted from the archive's
  headers.
- **Unsafe entries**: absolute paths, drive letters and `..` reject the zip;
  symlinks, directories and macOS metadata are skipped; encrypted entries
  reject it. Nested zips stay zips.
- **Checked at upload.** The files API refuses a zip that breaks a limit, so
  a stored zip always unpacks and its label is always true. Unpacking at
  message creation runs the same check again; a storage failure there (the
  Workspace quota, which unpacked files count toward) is logged like any copy.
- `.tar.gz` and Bashkit's own `unzip` reading standard archives are later
  work.

### 2. One intake path for every surface

A core `AttachmentIntake` in the channel runtime turns whatever a surface
received into stored attachments plus reference parts. Its input is bytes, a
`data:` URL, or a URL with an optional channel fetcher.

- **Channel fetchers** download files that need the channel's own credential,
  such as Slack `url_private` with the bot token (`files:read` scope).
- **Public URLs** are fetched by Everruns, not handed to the provider:
  `https` only, through the existing safe-URL validation, 30 second timeout.
- **Policy** per channel with defaults: 25 MiB per file, 10 files per message,
  allowed media types. The media type is checked against the bytes; a file
  whose bytes contradict a declared image or PDF type is stored as
  `application/octet-stream` and sent as a label only.
- **Failures become notes.** A file that cannot be stored becomes a text part
  such as `[Attachment report.pdf not received: 40 MB is over the 25 MB
  limit]`. Operators see the underlying error in logs.

| Surface | Change |
|---|---|
| Slack (server and embedded) | Every file in the triggering message goes through the Slack fetcher. Remote files (Google Drive and similar) keep their link in the text. |
| AG-UI, Public Chat | Inline binary, image and document input parts go through intake instead of `to_text()`; `imageIds` keeps working. Public Chat uses a stricter default policy. |
| A2A, PACT | `FilePart` with bytes or URI goes through intake. |
| Agent execution API, MCP `session_send_message`, serve | Accept file parts like REST messages. |
| Webhook, Poppy, voice | Unchanged; they carry no files today. |

### 3. Every attachment carries a label

When a message is turned into a model request, each attachment part becomes a
text label followed by its bytes when the model reads that media type:

```
Attached file /attachments/a1b2c3/report.pdf (application/pdf, 2.1 MB)
```

The label is plain text, so it survives everything that keeps text:

- Summarization and observation masking render the label for attachment
  parts instead of skipping them, and the summary prompt asks the summarizer
  to keep attachment paths it refers to.
- After any compaction that removed attachments from the request, the
  runtime appends a deterministic **attachment list** to the summary: one
  label per attachment the conversation references and the request no longer
  carries. It is built from stored history, not by the LLM, so it is complete.
- Infinity context's `query_history` returns labels with message text.
- Media types the model does not read (a model without vision, a CSV) are
  sent as the label only, so a model switch never sends bytes a provider
  rejects.

Labels appear on live sessions after this ships, which costs one prompt-cache
rewrite per session.

### 4. Files are resolved one at a time

The worker resolves each attachment on its own instead of in one batch capped
at 8 files and 11 MiB combined. One oversized or missing file becomes one
note for that file, not the loss of every PDF in the window. Server and
in-process workers then behave the same.

## Security

- Every fetch reuses the safe-URL validation (no private ranges, `https`
  only); channel fetchers send credentials only to their own host.
- Media types are verified against bytes before a provider sees them.
- `/attachments` is written only by the host; the model can read it, never
  write it.
- Message creation checks that every referenced upload belongs to the org.
- Threat model entries cover attacker-supplied URLs at intake, oversized or
  mislabeled files, and cross-org references.

## How eve does it (reference)

eve 0.77.0 stages every attachment into the sandbox under
`/workspace/.eve/attachments/<hash>/`, keeps a reference in history, labels
every file with its sandbox path, sends image and PDF bytes only within fixed
limits, downloads links itself (25 MB, 30 s, 10 per message) with a channel
`fetchFile` hook and an `uploadPolicy`, turns failures into notes, and lets
`read_file` reopen images and PDFs after compaction.

We take the label-with-path idea, the intake rules (own download, policy,
notes instead of failures) and the `read_file` reopen. We differ on where
bytes live: eve's copy sits in the sandbox, disappears with it and can be
overwritten by the agent; ours sits in the session file storage, which is
durable on the server, copied on fork, and read-only to the model.

## Rollout

1. **Labels, the attachment list after compaction, and one-at-a-time file
   resolution.** Fixes the compaction loss and the 8 file / 11 MiB cliff for
   today's uploads.
2. **Attachments in session file storage.** `/attachments` as a host-written
   root, uploads copied there at message creation, labels with paths.
   2b. `read_file` returns PDFs as native documents.
   2c. Zip attachments unpack to a read-only folder next to the archive.
3. **One intake path.** `AttachmentIntake` with policy and notes, then Slack,
   AG-UI and Public Chat, A2A and PACT, execution API and MCP.
