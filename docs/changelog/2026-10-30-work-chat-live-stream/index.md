---
title: Work Chat keeps the live conversation connected
description: The hosted chat no longer closes the live session stream after half a minute, which was showing up as a network error above the composer.
date: 2026-10-30
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/channels/#work-chat]
---

Work Chat follows a conversation over a live event stream. The hosted page
was closing that stream after 30 seconds. The browser then reported a
network error, and the next attempt could fail the same way, so the
conversation looked stuck even though the Agent was still working.

The stream now stays open until the conversation service cycles it, or
until you leave the page. A drop shows "Reconnecting to your conversation…"
and the chat resumes from the last event it received.

See [Work Chat](/features/channels/#work-chat).
