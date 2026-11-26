---
title: "Faster Mermaid diagrams across the UI"
description: "Mermaid diagrams now render with Pele throughout Everruns messages, descriptions, prompts, and Markdown file previews, with diagram zoom controls."
date: "2026-11-25"
products: [platform, cloud]
kind: improvement
significance: minor
surfaces: [ui]
links:
  docs: [/features/markdown-diagrams/]
---

Mermaid code blocks in chats, agent descriptions, prompts, and Markdown file previews now render with Pele. Diagrams have zoom controls, follow the light or dark theme, and adapt to their available space. See [Markdown diagrams](/features/markdown-diagrams/) for where to use them.

![A Mermaid flowchart rendered with Pele in the Everruns Markdown UI](./mermaid-diagram@2x.png)

For example, add this block to an agent reply or a Markdown file:

````markdown
```mermaid
flowchart LR
  Request --> Agent --> Reply
```
````
