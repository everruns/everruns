---
type: Decision
title: "Session Attachments"
description: "Design for keeping files people send to an agent usable for the whole session on every surface: one intake path that stores the bytes, a label with a stable path on every attachment, a read-only /attachments mount the agent can reopen after compaction, and a per-request byte budget instead of all-or-nothing resolution."
tags:
  - everruns
  - runtime-resources
  - attachments
  - compaction
  - channels
---
# Session Attachments

> Status: **Design, awaiting approval.** Nothing here is built. It changes how
> [Compaction](compaction.md), [Channels](../integrations/channels.md), the
> [Session Filesystem](file-store.md) and [Object Storage](object-storage.md)
> treat files that people attach to messages.

## Abstract

An attachment is a file a person (or a calling system) sends along with a
message: a screenshot in chat, a PDF in Slack, a `FilePart` over A2A. Today an
attachment is only a part of one message. The model sees it as long as that
message is in the request, and nothing else in the session knows it exists.

This design makes an attachment a session resource. Every surface stores the
bytes through one intake path and puts a reference into the message. The model
always sees a short label that names a stable path, such as
`/attachments/a1b2c3/report.pdf`. The bytes follow the label while they fit the
request; when compaction, a byte budget or a model without that modality leaves
them out, the label stays and the agent can open the path again with
`read_file` or the shell.

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
   open any of them again, whatever compaction, budgets or model switches did
   to the request.
3. Large attachments cost bytes in a request only while they are useful, and
   no single attachment can make the others disappear.
4. One mechanism for server, worker, Framework and serve hosts.

Non-goals: outbound files from the agent to a surface (the agent already
writes files to its workspace; posting them back is a channel feature), text
extraction for office formats, and audio or video understanding.

## Design

### 1. An attachment is a stored session resource

An attachment record holds an id, org, the session that received it, file
name, verified media type, size, SHA-256, the surface it came from and the
bytes. Messages carry a reference part, never bytes.

- **Stores.** Images keep the `images` store (thumbnails, object storage
  offload already exist). The `files` store stops being PDF-only: it accepts
  any media type and gains the same `BlobStore` offload images have, so a
  25 MB file is not a Postgres `bytea`. The `file` content part keeps its
  shape (`file_id`, `filename`) and simply allows any type.
- **Inline bytes are stored at the door.** An inline base64 `image` part (REST,
  AG-UI) is written to the image store at message creation and replaced by an
  `image_file` part, so event rows and their full-text index stop carrying
  base64. Inline public `https` URLs stay URLs only for images the provider
  fetches itself; everything else is downloaded at intake (section 2).
- **Ownership is checked at creation.** Message creation verifies that every
  referenced `image_id` / `file_id` belongs to the org. Today only resolution
  is org-scoped.
- **Lifetime.** An attachment lives as long as any session that references it
  (forks add references; deleting the last session or the org deletes it),
  following the [object storage garbage collection](object-storage.md#garbage-collection)
  rules.

### 2. One intake path for every surface

A core `AttachmentIntake` in the channel runtime turns whatever a surface
received into stored attachments plus reference parts. Its input is one of:
bytes, a `data:` URL, or a URL with an optional channel fetcher.

- **Channel fetchers** download files that need the channel's own credential:
  Slack `url_private` with the bot token (needs the `files:read` scope),
  later Teams or Telegram. A fetcher can return "not available, because …".
- **Public URLs** are fetched by Everruns, not handed to the provider: `https`
  only, through the existing safe-URL validation, 30 second timeout, one at a
  time. A private or expired link becomes a note instead of a provider error
  on every later turn.
- **Upload policy** per channel, with defaults: 25 MiB per file, 10 files per
  message, allowed media types. The media type is checked against the bytes
  (magic numbers); a file whose bytes contradict its declared image or PDF
  type is stored as `application/octet-stream`.
- **Failures become notes.** A file that cannot be stored becomes a text part
  such as `[Attachment report.pdf not received: 40 MB is over the 25 MB
  limit]`. Operators see the underlying error in logs.

Surface by surface:

| Surface | Change |
|---|---|
| Slack (server and embedded) | Every file in the triggering message, and files posted in the thread since the agent's last reply, go through the Slack fetcher. Remote files (Google Drive and similar) keep their link in the text. |
| AG-UI, Public Chat | Inline binary/image/document input parts go through intake instead of `to_text()`; `imageIds` keeps working. Public Chat uses a stricter default policy (anonymous visitors). |
| A2A | `FilePart` with bytes or URI goes through intake. PACT accepts file parts under the same policy. |
| Agent execution API, MCP `session_send_message`, serve | Accept `image_file` / `file` references and inline file parts, same as REST messages. |
| Web chat | Any allowed type can be attached; images and PDFs keep previews. The client limit is read from the server instead of a hard-coded 32 MB. |
| Webhook, Poppy, voice | Unchanged for now. They have no file payloads today; when they do, they use intake. |

### 3. Every attachment carries a label

When a message is turned into a model request, each attachment part becomes a
text label followed, when it fits (section 5), by the bytes:

```
Attached file /attachments/a1b2c3/report.pdf (application/pdf, 2.1 MB)
```

The label is plain text, so it survives everything that keeps text:

- `extract_text` used by summarization and observation masking renders the
  label for attachment parts instead of skipping them, and the summary prompt
  asks the summarizer to keep attachment paths it refers to.
- After any compaction that removed attachments from the request, the runtime
  appends a deterministic **attachment index** to the summary: one label per
  attachment the conversation still references and the request no longer
  carries. It is built from stored history, not by the LLM, so it is complete.
- Infinity context's `query_history` returns labels with message text.
- Native provider compaction keeps working as today; the index is added after
  its output in our own message list.

Labels appear on live sessions after this ships, which costs one prompt-cache
rewrite per session; after that they are stable.

### 4. A read-only `/attachments` mount

The session filesystem gains a read-only mount at `/attachments`, added through
`MountFs::with_mount` like any other root (see
[Session Filesystem](file-store.md)). It lists the attachments referenced by
this session's history as `/attachments/<short-id>/<filename>` and reads bytes
from the attachment stores.

- **Why a mount and not a copy in `/workspace`:** the bytes already live in a
  durable org store; a copy would count against the workspace quota, could be
  overwritten or deleted by the agent (eve has to detect that and answer
  "file not found"), and would be lost with a sandbox. A mount costs nothing
  until read and forks get it for free.
- **`read_file`** already returns PNG, JPEG, GIF and WebP as image content. It
  gains PDFs as file content (up to the model's PDF limit) when the model
  accepts PDFs, and a clear metadata answer otherwise. Other types read as
  text or binary metadata as today.
- **Shell and sandboxes.** Bashkit sees the mount directly. Container
  sandboxes (Daytona, Modal, Docker) materialize an attachment into the
  sandbox at the same path the first time the agent touches it, or on sandbox
  start for attachments from the current turn, so a script can run
  `pdftotext /attachments/a1b2c3/report.pdf`.

### 5. A per-request byte budget instead of all-or-nothing resolution

The model view decides per attachment whether its bytes go into a request:

- Bytes go only for media types the target model accepts (catalog modalities:
  image, PDF). Anything else is label only, so a model switch never sends
  bytes the provider rejects.
- Within that, newest first, up to a per-request budget: a count cap below the
  provider's limit (Anthropic allows 100 images) and a total byte cap.
  Attachments that do not fit are label only.
- **Sticky decisions.** Once an attachment has been sent as bytes it keeps
  being sent until compaction removes its message, so the prompt prefix and
  the provider cache stay stable; the budget only decides for newly arriving
  attachments.
- **Per-attachment resolution.** The worker resolves only the attachments
  that will be sent, one at a time, by presigned URL with the gRPC byte
  fallback, as images already do. One oversized or missing PDF becomes one
  `File not found` note, not the loss of every PDF.
- Token estimates count image and PDF parts by the provider's published
  per-image and per-page costs instead of 50 characters, so proactive
  compaction triggers on time.

### 6. Delegation passes attachments

The subagent and handoff tools take an optional `attachments` list of
`/attachments/...` paths. The child's first message carries the same
reference parts, and the child session gets its own `/attachments` mount. No
bytes are copied; ownership is the same org.

### 7. Tool-produced files (later)

Images and files that tools return (screenshots, MCP image or blob content)
follow the same rule in a later step: above a small size they are stored as
attachments, the tool result keeps the label, and observation masking leaves
the label behind. Until then today's 5 MiB per-tool-result image cap stays.

## Security

- Every fetch reuses the safe-URL validation (no private ranges, `https`
  only); channel fetchers send credentials only to their own host.
- Media types are verified against bytes before a provider sees them.
- `/attachments` is read-only and lists only attachments referenced by the
  session, and resolution stays org-scoped. Public Chat visitors get a
  separate, smaller policy.
- Moving inline base64 out of `events.data` removes attachment bytes from the
  plaintext event table and its search index.
- New threat model entries cover attacker-supplied URLs at intake, oversized
  or mislabeled files, and cross-org references.

## How eve does it (reference)

eve 0.77.0 stages every attachment into the sandbox under
`/workspace/.eve/attachments/<hash>/`, keeps a reference in history, labels
every file with its sandbox path, sends image and PDF bytes only within fixed
limits, downloads links itself (25 MB, 30 s, 10 per message) with a channel
`fetchFile` hook and an `uploadPolicy`, turns failures into notes, and lets
`read_file` reopen images and PDFs after compaction.

We take the label-with-path idea, the intake rules (own download, policy,
notes instead of failures) and the `read_file` reopen. We differ on where
bytes live: eve's copy disappears with the sandbox and can be overwritten by
the agent; ours stays in the org store behind a read-only mount. eve sends the
same bytes on every call; we add a sticky per-request budget so long sessions
do not grow without bound.

## Rollout

Each step is one PR and useful on its own.

1. **Labels and the attachment index.** Labels on every attachment part,
   summarizer and masking keep them, the deterministic index after
   compaction, and per-attachment file resolution (removes the 8 file / 11 MiB
   cliff).
2. **`/attachments` mount** in the session filesystem, `read_file` for PDFs,
   sandbox materialization.
3. **Intake path and Slack.** `AttachmentIntake`, upload policy, the Slack
   fetcher, generic media types and object storage offload in the `files`
   store. This delivers the "PDF in Slack, reopen after compaction" scenario.
4. **Other surfaces.** AG-UI and Public Chat inline parts, A2A and PACT file
   parts, execution API and MCP references, web chat any-type upload.
5. **Request budget.** Modality check, image count and byte caps, sticky
   decisions, better token estimates; inline base64 offload at message
   creation and the ownership check.
6. **Delegation** `attachments` parameter.
7. **Tool-produced files** as attachments.

## Open questions

- Default per-file cap: 25 MiB matches eve and Slack's common use; the PDF
  model limit is lower (about 32 MB per request on Anthropic). The budget in
  section 5 handles the difference, but the cap could also be per org plan on
  hosted.
- Whether `/attachments` should also list attachments from the parent session
  for a forked or delegated child automatically, or only the ones passed.
