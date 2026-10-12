---
title: "Sandbox file tools all read the managed Sandbox workspace"
description: "With a managed Sandbox, read_many_files and list_directory now read the Sandbox's /workspace, so files the agent cloned there no longer read as not found."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/features/sandbox-templates/]
  prs: []
---

On a Sandbox Worker with a managed Sandbox such as Daytona, `read_file` read the
Sandbox while `read_many_files` and `list_directory` still read the Session's
own files. A repository the agent had just cloned looked missing:

```json
{
  "path": "/workspace/yolop/AGENTS.md",
  "error": "File not found: /workspace/yolop/AGENTS.md"
}
```

Every file tool now works on the Sandbox's `/workspace`, and tools that only
reach the Session's files are no longer offered there. Skills and saved tool
output stay readable through a fallback. See
[Sandbox Templates](/features/sandbox-templates/).
