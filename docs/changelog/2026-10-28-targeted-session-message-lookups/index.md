---
title: Long sessions avoid repeated message-history reads
description: Individual message reads now use a session-scoped index, reducing turn startup work and keeping older inputs accessible beyond the history window.
date: 2026-10-28
kind: fix
significance: minor
products: [platform, cloud]
release: "0.50.0"
surfaces: [api, operations]
---

Looking up one message during turn startup no longer loads and parses the
session's recent conversation. The runtime and both worker transports now read
the requested message directly, and input-message lookups use an index instead
of scanning unrelated history.

This also fixes reads of older inputs that fell outside the 2,000-message history
window. Session isolation and legacy event records remain supported. No API or
client changes are required; self-hosted installations apply the new database
index through the normal upgrade migration.
