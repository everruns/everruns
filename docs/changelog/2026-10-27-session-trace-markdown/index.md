---
title: Session trace messages render as Markdown
description: Session trace model messages and answers now render Markdown by default, with a plain text option for inspecting the original source.
date: 2026-10-27
kind: improvement
significance: minor
products: [platform, cloud]
release: "0.50.0"
surfaces: [ui]
links:
  docs: [/features/ui/]
---

Session trace model messages and answers now render Markdown by default, making headings, lists, links, and code easier to read. Choose **Plain text** in the trace controls to inspect the original message source; choose **Markdown** to return to the formatted view.

![Session trace with a formatted answer and Markdown and plain text controls](./session-trace@2x.png)

The choice applies to the loaded trace, including expanded sub-agent steps, and stays in the page URL so it can be shared. Learn more about the [Web UI](/features/ui/).
