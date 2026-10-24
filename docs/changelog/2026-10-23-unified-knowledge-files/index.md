---
title: Knowledge unifies files and optional indexed search
description: Mount notes, source trees and binary originals through one Knowledge feature, with Markkit text extraction and optional keyword or hybrid retrieval.
date: 2026-10-23
kind: feature
significance: notable
products: [platform]
surfaces: [ui, api, agent, cli]
links:
  docs: [/capabilities/knowledge/]
---

[Knowledge](/capabilities/knowledge/) replaces the two undeployed Knowledge Base and Knowledge Index surfaces. Create a file collection, add notes or uploads, or connect GitHub through an existing connection. Public HTTPS Git sources also work. Indexing can stay off while agents browse read-only mounts and use exact grep.

![Knowledge file catalog with a binary DOCX text preview and per-file access controls](./knowledge@2x.png)

Original binaries and file hierarchy are retained. Markkit extracts HTML, DOCX and spreadsheets; unsupported formats remain downloadable. Enable keyword or hybrid search when needed, and use the same retrieval service through an agent tool or Bashkit:

```bash
knowledge search --limit 5 'refund policy'
```

Collections preserve frontmatter and annotations, update indexes incrementally, and return immutable citations. Organization search accounts use encrypted service credentials. The feature retains its development rollout grade. Shared workspaces keep Knowledge attachments session-scoped; unsupported binary formats retain their originals.
