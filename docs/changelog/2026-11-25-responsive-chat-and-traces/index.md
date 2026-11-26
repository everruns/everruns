---
title: "Chat and traces adapt to narrow panes"
description: "Read long messages on phones and beside pages, with a focused trace timeline and on-demand turn navigation and step details."
date: 2026-11-25
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/ui/]
---

Chat wraps long messages, task names and automatic reports within phone,
floating and docked panels. Code blocks keep their own horizontal scrolling.

Trace now follows its pane's width when Chat is open beside it. Narrow panes
keep the timeline visible: **Turns** opens navigation, **Filters** reveals
secondary controls, and selecting a step opens a full-height detail drawer.
Return with **Back to trace** or Escape without losing your place. Wider panes
show details and turn navigation alongside the timeline.

Session metadata and actions remain available under **Session details** in
narrow panes. [The UI guide](/features/ui/) describes session inspection.

![Trace in a narrow pane, with Turns navigation and filters above the timeline](./trace-narrow@2x.png)
