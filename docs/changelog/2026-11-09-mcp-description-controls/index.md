---
title: Invisible controls are visible in MCP descriptions
description: MCP descriptive metadata uses visible Unicode markers while tool names, canonical schemas and invocation values stay unchanged.
date: 2026-11-09
kind: improvement
significance: minor
products: [framework, platform, cloud]
release: "0.52.0"
surfaces: [agent]
links:
  docs: [/features/mcp/]
---

MCP tool and server descriptions now display a narrow set of invisible and
bidirectional formatting controls as markers such as `[U+202E]`. This also applies
to tool display titles, deferred server descriptions, and recognized schema
`title` and `description` annotations sent to models or returned by tool search.

Canonical schemas, tool names and argument values stay unchanged. Joiners used
by scripts and emoji remain intact. Schema annotation traversal has depth and
node limits, and leaves opaque extension data unchanged. This is metadata
hygiene, not a guarantee against prompt injection.

For example, this [MCP tool description](/features/mcp/) contains a right-to-left
override in its source JSON:

```text
Source JSON: {"description": "Search\u202e documentation"}
Model text: Search[U+202E] documentation
```
