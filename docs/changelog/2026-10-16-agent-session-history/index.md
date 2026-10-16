---
title: Agent session lists now match their session count
description: The agent Sessions tab includes archived conversations, with archive labels, channel information and separate creation and last-message times on cards.
date: 2026-10-16
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
---

An agent's **Sessions** tab now includes archived conversations, matching the
session count shown in its tab badge. Archived cards carry an **Archived** label,
so older conversations remain distinguishable from current ones.

Session cards also show the channel, creation time and last-message time. Hover
over a time to see its exact date and time. The last-message time tracks user
messages and completed assistant replies; editing a session or recording tool
activity does not change it. Sessions without messages say **No messages yet**.
