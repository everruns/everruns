---
type: Test Case
title: "TC001: Anonymous chat and isolation boundary"
description: "Verify that an anonymous visitor can use one published public chat while its UI, requests, bootstrap data, errors, and conversation identity remain isolated from console and organization data."
tags:
  - everruns
  - test-case
  - ui
  - public-chat
  - security
---
# TC001: Anonymous chat and isolation boundary

## Description

Verify that an anonymous visitor can use one published public chat while its UI, requests, bootstrap data, errors, and conversation identity remain isolated from console and organization data.

## Preconditions

- The full stack is running with full authentication and `public_chat` enabled.
- Organization A has a published app with an enabled anonymous Public Chat channel, no shared token, and distinct name, logo, color, and welcome message.
- The app's agent can answer a deterministic two-turn prompt.
- Organization A also has one unpublished app and one app with a disabled Public Chat channel.
- Organization B contains unique app, agent, session, and file names that do not appear in Organization A.
- The tester has a private browser window with no Everruns platform session.

## Test Data

| Field | Value |
|---|---|
| Public route | `/public-chat/{publishedAppId}` |
| First message | `Remember the code EVE-995 and reply ready.` |
| Follow-up | `What code did I ask you to remember?` |
| Unavailable IDs | Random, unpublished, and disabled-channel app IDs |
| Org B marker | A unique name from Organization B |

## Steps

1. In the unauthenticated private window, open the published public route.
2. Confirm the page shows only the configured name, logo, color, welcome message, conversation area, and composer; verify there is no console sidebar, organization switcher, app or agent picker, settings link, or session/file navigation.
3. Inspect the bootstrap request to `/api/v1/apps/{publishedAppId}/public-chat/config`.
4. Confirm the JSON contains only public app branding, anonymous/sign-in mode, and public CAPTCHA configuration when enabled; verify it contains no organization ID, agent or harness ID, shared token, CAPTCHA secret, auth secret, session ID, internal tool name, or provider/model configuration.
5. Send the first message and verify the reply streams into one assistant message.
6. Send the follow-up and verify the reply uses the first turn's context.
7. Confirm network traffic for both turns targets only `/api/v1/apps/{publishedAppId}/public-chat` and does not call console organization, app-list, agent-list, session-list, file, or tool endpoints.
8. Record `everruns_public_chat_thread:{publishedAppId}` from local storage, refresh, and confirm the same per-app thread ID remains.
9. Send another message, click **New conversation**, and confirm messages clear and the stored thread ID changes.
10. Attempt direct unauthenticated requests to console list endpoints for organizations, apps, agents, sessions, and files; confirm none returns Organization A or B data.
11. Search all public UI text and public endpoint responses for the Organization B marker and confirm it is absent.
12. Open the public route with the random, unpublished, and disabled-channel IDs and compare the failed bootstrap requests and page messages.

## Expected Result

- Anonymous access works without a platform login and streams a contextual two-turn conversation.
- The rendered surface exposes one app only and provides no path into the Everruns console.
- Bootstrap and turn requests disclose no secret, organization-scoped, model/provider, tool, or internal entity data.
- Anonymous console API requests do not return organization, app, agent, session, or file data.
- Conversation identity is stable per browser and app until **New conversation** rotates it.
- Random, unpublished, and disabled-channel IDs all return the same sanitized 404-class availability behavior without revealing which resource exists.
