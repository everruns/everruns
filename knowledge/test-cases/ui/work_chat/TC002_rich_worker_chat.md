---
type: Test Case
title: "TC002: Work Chat - Rich results and conversation controls"
description: Verify structured widgets, per-turn activity, artifacts, consumer history and revocable read-only links.
---
# TC002: Work Chat - Rich results and conversation controls

## Preconditions

Publish a multi-chat Work Chat channel with workspace tools and OpenUI/A2UI
capabilities. Configure one permitted model. Use two authenticated consumers.
Keep the existing consumer-isolation case as the authentication prerequisite.

## Steps

1. Create two conversations. Ask for a chart and a tabbed report. Verify charts
   and tabs render inline, the message copy action works, and each reply has its
   own work log. A malformed widget must retain readable source.
2. Trigger a widget message action. Verify it reaches the current conversation
   through the consumer API; a generated unsafe URL must not execute.
3. Attach multiple files by selection, paste and drop. Verify three files totaling
   at most 4 MB are accepted and larger batches are refused before sending.
4. Open Markdown, CSV, an image and HTML in Workspace. Filter CSV rows, download
   a text file, switch to source editing, and verify stale edits still conflict.
   Verify HTML scripts and external requests remain blocked.
5. Search, rename, pin and archive a conversation. Reload, find it under archived
   history, reopen and send. Another consumer must not read or mutate it. The
   permanent main conversation in single mode must retain its lifecycle.
6. Select the publisher-permitted model and effort. Verify an unlisted model is
   rejected by the API. Other execution channel types retain their model policy.
7. Publish a read-only link. Open it signed out. Verify messages/widgets render,
   with no composer, workspace, pending approvals or message actions. Revoke the
   link and confirm it fails. Unpublishing the channel also removes access.

## Expected Result

The channel provides the worker interface using existing execution services.
Rich output preserves consumer, workspace, approval and public-share boundaries.
Rendering fixtures do not establish real identity-provider or Agent execution.
