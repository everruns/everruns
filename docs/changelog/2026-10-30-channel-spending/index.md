---
title: Channel spending is one section
description: The shared channel budget and the per-person daily limit sit together under Spending on the channel page. Save the channel to keep the daily limit.
date: 2026-10-30
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
visual: Channel page Spending section with the shared pool and the per-person daily limit
links:
  docs: [/features/channels/#work-chat]
---

The channel page shows spending in one place. **Spending** holds the shared channel budget and, on Work Chat and Agent API, the daily limit for each person.

The shared pool covers everyone on the channel. Sessions stop when it runs out. The daily limit caps one person until midnight UTC. Leave that field empty for no per-person cap. Saving the channel keeps the daily limit. Adding or editing the shared pool still applies immediately.

A rate limit stays under **Advanced** on Work Chat, and with the other Agent API settings. See [Work Chat](/features/channels/#work-chat).
