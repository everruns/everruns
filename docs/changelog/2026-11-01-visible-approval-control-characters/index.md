---
title: "Tool approvals reveal hidden bidi controls"
description: "Approval cards show directional and control characters as visible Unicode escapes, helping you inspect tool names and arguments before consenting."
date: 2026-11-01
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui]
---

Tool approval cards now reveal hidden directional and control characters before
you decide. For example, a filename containing a right-to-left override shows a
highlighted `\u202E` marker instead of letting that character reorder the preview.
Markers include a code point label for screen readers.

The same display applies to remote MCP approvals and credential consent details.
Persian text, joiners and emoji keep their normal appearance. This changes only
what the card displays: original arguments, saved approval bindings and tool
execution remain unchanged.

![Tool and MCP approvals highlight directional controls while preserving Persian text and emoji](./approval-controls@2x.png)
