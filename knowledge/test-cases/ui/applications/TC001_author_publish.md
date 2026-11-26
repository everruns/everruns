---
type: Test Case
title: "TC001: Applications - Author and publish"
description: Verify Application authoring, stable publication and optional sidebar integration.
---
# TC001: Applications - Author and publish

## Description
Verify Application authoring, stable publication and optional sidebar integration.

## Preconditions
Canonical local stack with configured CHAT_DOMAIN, an organization administrator,
and an available Agent. Local none mode is acceptable for authoring only.

## Test Data

| Field | Value |
| --- | --- |
| Name | Content Studio |
| Stable name | content-studio-<unique run suffix> |
| Resources | Writing assistant, Review assistant, Editorial room, Brand handbook |

## Steps
1. Open Build → Applications; create the named Application. Record its ID.
2. Create two Chats using the Agent, one Room and one organization Memory.
3. Rename/reorder navigation entries, save, reload and verify persisted bindings.
4. Select invited audience and enter two verified email addresses; save and reload.
5. Enable the sidebar launcher and publish. Verify the stable public URL and launcher.
6. Try changing the stable name through the API; verify rejection and unchanged name.
7. Unpublish and verify the launcher disappears. Archive and verify the name remains reserved.

## Expected Result
All resources belong to this Application, configuration persists, publication and
launcher visibility follow current state. The stable name never changes or becomes
available for reassignment. No legacy Apps page or API is introduced.
