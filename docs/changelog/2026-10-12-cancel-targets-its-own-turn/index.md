---
title: "A cancel no longer stops the next message's turn"
description: "Cancelling a turn now ends only that turn. A message sent after the cancel always gets its own answer, even when the worker failed mid-cancel."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/event-reference/]
---

Before this fix, a cancel could outlive the turn it was meant for. When the
worker that recorded `turn.cancelled` failed before it handed the session back,
for example on a database timeout, the next worker applied the same cancel to
the turn of the next message, which ended as `turn.cancelled` a few
milliseconds after it started. A message sent after a cancel whose worker had
crashed could also be dropped along with the cancelled turn.

A cancel now records which input it was requested for and ends only that turn.
Messages sent after the cancel start their own turn and are answered.
